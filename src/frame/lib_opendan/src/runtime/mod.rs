pub mod bin_overlay;
pub use agent_tool::runtime::{native_host_id, AgentRuntime, NativeRuntime};
pub use bin_overlay::{BinPlan, ToolPlan};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use agent_tool::runtime::node::{NodeExecutorConnector, RemoteNodeTarget, REMOTE_NODE_KIND};
use agent_tool::runtime::{RuntimeConfig, RuntimeOpenCtx};
use agent_tool::xllm::{LoopModel, ToolsConfig, XllmDeps};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

static NODE_CONNECTOR: RwLock<Option<Arc<dyn NodeExecutorConnector>>> = RwLock::new(None);

/// Install the connector this process reaches Node Executors with (tests,
/// hosts with their own transport). Default: the BuckyOS runtime's gateway.
pub fn set_node_connector(connector: Arc<dyn NodeExecutorConnector>) {
    *NODE_CONNECTOR.write().expect("node connector") = Some(connector);
}

pub fn node_connector() -> Arc<dyn NodeExecutorConnector> {
    NODE_CONNECTOR
        .read()
        .expect("node connector")
        .clone()
        .unwrap_or_else(|| Arc::new(agent_tool::runtime::node::BuckyosNodeConnector))
}

/// The `remote_node` configuration a node runtime's registration carries.
pub fn node_config_of(runtime_id: &str, exec: &serde_json::Value) -> Result<RuntimeConfig> {
    let config: RuntimeConfig = serde_json::from_value(exec.clone())
        .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: invalid exec: {e}")))?;
    if config.kind() != REMOTE_NODE_KIND {
        return Err(OpenDanError::Bind(format!(
            "runtime `{runtime_id}` executes through {} and cannot host a Session yet",
            config.kind()
        )));
    }
    let node = config.remote_node.as_ref();
    if node.and_then(|n| n.node_did.as_deref()).is_none_or(str::is_empty)
        || node.and_then(|n| n.profile_id.as_deref()).is_none_or(str::is_empty)
    {
        return Err(OpenDanError::Bind(format!(
            "runtime `{runtime_id}`: remote_node needs node_did and profile_id"
        )));
    }
    Ok(config)
}

/// Open a node runtime as a target directory (workspace placement, the
/// default session directory): login + describe through the connector.
pub async fn open_node_target(record: &RuntimeRecord, ops_dir: &Path) -> Result<RemoteNodeTarget> {
    let exec = record
        .exec
        .as_ref()
        .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` is not enabled", record.runtime_id)))?;
    let config = node_config_of(&record.runtime_id, exec)?;
    open_node_target_of(&record.runtime_id, &config, ops_dir).await
}

async fn open_node_target_of(runtime_id: &str, config: &RuntimeConfig, ops_dir: &Path) -> Result<RemoteNodeTarget> {
    let node = config.remote_node.clone().unwrap_or_default();
    RemoteNodeTarget::open(
        node_connector(),
        node.node_did.as_deref().unwrap_or_default(),
        node.profile_id.as_deref().unwrap_or_default(),
        runtime_id,
        ops_dir.to_path_buf(),
    )
    .await
    .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: {e}")))
}

/// Join path parts below `base` with the separator of the target OS (a
/// drive path is a Windows path whatever this side runs on).
pub fn target_join(base: &str, parts: &[&str]) -> String {
    let windows = buckyos_api::is_absolute_node_path(base) && !base.starts_with('/');
    let sep = if windows { '\\' } else { '/' };
    let mut out = base.trim_end_matches(['/', '\\']).to_string();
    for part in parts {
        out.push(sep);
        out.push_str(part);
    }
    out
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionEnvCtx {
    pub session_id: String,
    pub session_dir: PathBuf,
    pub agent_did: String,
    pub agent_root: Option<PathBuf>,
    pub input_queue: Option<String>,
    pub trace_id: String,
    pub extra_env: Vec<(String, String)>,
    /// The session's frozen cmd_skills: their commands
    /// (`<agent_root>/tools/<name>/bin`) come first on PATH.
    pub cmd_skills: Vec<String>,
    /// The session's workspace: where it is, beside the working directory.
    pub workspace: Option<WorkspaceBinding>,
}

/// `<agent_root>/tools/<name>/bin` of each cmd_skill that brings commands.
pub fn skill_tool_dirs(agent_root: &Path, cmd_skills: &[String]) -> Vec<PathBuf> {
    cmd_skills
        .iter()
        .map(|n| agent_root.join(crate::skills::TOOLS_DIR).join(n).join("bin"))
        .filter(|d| d.is_dir())
        .collect()
}

/// The names of the session's frozen cmd_skills.
pub fn frozen_cmd_skills(cfg: &SessionConfig) -> Vec<String> {
    cfg.prompt
        .frozen
        .as_ref()
        .map(|f| f.cmd_skills.iter().map(|c| c.name.clone()).collect())
        .unwrap_or_default()
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
    let native = is_native_runtime(&record.location.runtime_id);
    if record.lifecycle != WorkspaceLifecycle::Active
        || record.availability != WorkspaceAvailability::Available
        || record.runtime_host != native_host_id()
    {
        return Err(OpenDanError::WorkspaceBindingInvalid(format!(
            "workspace {} is not available for a session: {:?} {:?}",
            record.workspace_id, record.lifecycle, record.availability
        )));
    }
    if native {
        verify_workspace_access(&record.location.directory, workspace.access)?;
    }
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
            {
                return Err(invalid(format!(
                    "workspace {} no longer matches location revision {} ({:?}, {:?})",
                    binding.workspace_id,
                    binding.location_revision,
                    record.lifecycle,
                    record.availability
                )));
            }
            if is_native_runtime(&record.location.runtime_id) {
                verify_workspace_access(&binding.location.directory, binding.access)?;
                let actual = std::fs::canonicalize(&binding.location.directory)
                    .map_err(|e| invalid(e.to_string()))?;
                if actual != binding.location.directory {
                    return Err(invalid(
                        "the workspace directory changed its canonical location".into(),
                    ));
                }
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

/// The control-side working directory of a session: its session directory
/// (Q4: choosing a workspace never moves the default location). A session
/// on a node runtime works in its directory there ([`target_workdir`]).
pub fn resolve_workdir(sd: &SessionDir) -> Result<PathBuf> {
    sd.path()
        .canonicalize()
        .map_err(|e| OpenDanError::Bind(format!("{}: {e}", sd.path().display())))
}

/// The working directory of a node-runtime session on its node, once the
/// first binding created it (`None` before).
pub fn target_workdir(sd: &SessionDir) -> Result<Option<String>> {
    Ok(sd.binding_opt()?.map(|b| b.workdir))
}

/// The workspace of a session as its prompt and helpers see it: its runtime
/// and directory there, and whether that is the session-runtime (then a
/// `cd` reaches it; otherwise files are copied between the two).
pub fn workspace_view(cfg: &SessionConfig) -> Option<serde_json::Value> {
    let b = cfg.workspace_binding.as_ref()?;
    let runtime_id = canonical_runtime(&b.location.runtime_id);
    Some(serde_json::json!({
        "workspace_id": b.workspace_id,
        "runtime_id": runtime_id,
        "directory": b.location.directory,
        "access": b.access,
        "same_runtime": runtime_id == cfg.runtime.runtime_id(),
    }))
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
/// working directory is the session's working directory, the baseline the
/// session-runtime's declared one (a node runtime has both from
/// [`runtime_for_session`]).
fn settle_for_session(
    sd: &SessionDir,
    cfg: &SessionConfig,
    config: &mut agent_tool::runtime::RuntimeConfig,
) -> Result<()> {
    if cfg.runtime.is_node() {
        // The node directory was settled when the runtime was built; a
        // template may only add environment.
        if config.kind() != REMOTE_NODE_KIND {
            return Err(OpenDanError::Bind(format!(
                "Config: the session is frozen to node runtime `{}`; a behavior cannot declare runtime kind `{}`",
                cfg.runtime.runtime_id(),
                config.kind()
            )));
        }
        if config.workdir.is_none() {
            return Err(OpenDanError::Bind("the node session has no working directory yet".into()));
        }
        return Ok(());
    }
    if let Some(frozen) = &cfg.runtime.frozen {
        config.baseline = Some(frozen.baseline);
    }
    let expected = resolve_workdir(sd)?;
    if let Some(workdir) = &config.workdir {
        let path = Path::new(workdir);
        if !path.is_absolute() || path.canonicalize().ok().as_ref() != Some(&expected) {
            return Err(OpenDanError::Bind(
                "an explicit runtime workdir must be the session's working directory".into(),
            ));
        }
    }
    config.workdir = Some(expected.display().to_string());
    Ok(())
}

/// The runtime of a session, built from what it was frozen to: what a host
/// puts into that session's `RunnerDeps.runtime`. A session on a node
/// runtime gets its `remote_node` configuration from the frozen selection
/// (never from the registry again) and its working directory on the node,
/// created under the node's roots on the first drive (a workspace never
/// takes its place). Nothing else is opened here; that happens at the
/// binding, under the session lease.
pub async fn runtime_for_session(
    sd: &SessionDir,
    cfg: &SessionConfig,
    agent: &dyn crate::state::AgentStateClient,
    runtime_id: Option<&str>,
    deps: &XllmDeps,
) -> Result<Arc<dyn AgentRuntime>> {
    if let Some(frozen) = cfg.runtime.frozen.as_ref().filter(|f| f.kind.is_node()) {
        let exec = frozen.exec.as_ref().ok_or_else(|| {
            OpenDanError::Bind(format!("runtime `{}` was frozen without an executor", frozen.runtime_id))
        })?;
        let mut config = node_config_of(&frozen.runtime_id, exec)?;
        config.id = Some(frozen.runtime_id.clone());
        if let Some(declared) = declared_runtime(sd, cfg)? {
            if declared.kind.is_some() && declared.kind() != REMOTE_NODE_KIND {
                return Err(OpenDanError::Bind(format!(
                    "Config: the session is frozen to node runtime `{}`; a behavior cannot declare runtime kind `{}`",
                    frozen.runtime_id,
                    declared.kind()
                )));
            }
            config.env.extend(declared.env);
        }
        config.baseline = Some(frozen.baseline);
        config.workdir = Some(node_workdir(sd, agent, &frozen.runtime_id, &config).await?);
        let mut deps = deps.clone();
        if deps.node_executor.is_none() {
            deps.node_executor = Some(node_connector());
        }
        return agent_tool::runtime::RuntimeRegistry::from_config_with(&config, &deps)
            .map_err(|e| OpenDanError::Bind(e.to_string()));
    }
    let mut config = declared_runtime(sd, cfg)?.unwrap_or_default();
    if config.kind() == REMOTE_NODE_KIND {
        return Err(OpenDanError::Bind(
            "Config: a behavior declares remote_node, but the session was not frozen to a node runtime; choose the runtime at creation (`--runtime`)".into(),
        ));
    }
    // An explicit id is a binding identity requirement, not a way to choose
    // the executor.
    if let Some(id) = runtime_id {
        config.id = Some(id.to_string());
    }
    if config.id.is_none() {
        config.id = cfg.runtime.requirement.runtime_id.clone();
    }
    settle_for_session(sd, cfg, &mut config)?;
    agent_tool::runtime::RuntimeRegistry::from_config(&config)
        .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// Where a node-runtime session works on its node: what is bound, else
/// `<first root>/.opendan/<agent>/sessions/<sid>`, created now (the
/// executor's `mkdir`, idempotent).
async fn node_workdir(
    sd: &SessionDir,
    agent: &dyn crate::state::AgentStateClient,
    runtime_id: &str,
    config: &RuntimeConfig,
) -> Result<String> {
    if let Some(known) = target_workdir(sd)? {
        return Ok(known);
    }
    let target = open_node_target_of(runtime_id, config, &sd.state_dir().join("runtime_ops")).await?;
    let root = target.workdir_roots().first().cloned().ok_or_else(|| {
        OpenDanError::Bind(format!("runtime `{runtime_id}`: the node grants no working directory root"))
    })?;
    let dir = target_join(&root, &[".opendan", agent.agent_id(), "sessions", sd.sid()]);
    target
        .mkdir(&dir)
        .await
        .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: create {dir}: {e}")))
}

/// The runtime a drive uses: the provided one, checked against what the
/// session declares and completed by the session rules. A provided runtime
/// that names another executor than the session's is refused.
pub fn session_runtime(
    sd: &SessionDir,
    cfg: &SessionConfig,
    provided: &std::sync::Arc<dyn AgentRuntime>,
) -> Result<std::sync::Arc<dyn AgentRuntime>> {
    let original = provided.config();
    let mut config = original.clone();
    if let Some(r) = declared_runtime(sd, cfg)? {
        if cfg.runtime.is_node() {
            // The node target is the session's; a behavior adds environment.
            if r.kind.is_some() && r.kind() != REMOTE_NODE_KIND {
                return Err(OpenDanError::Bind(format!(
                    "Config: the session is frozen to node runtime `{}`; a behavior cannot declare runtime kind `{}`",
                    cfg.runtime.runtime_id(),
                    r.kind()
                )));
            }
            config.env.extend(r.env.clone());
        } else {
            config.merge_over(&r);
        }
    }
    settle_for_session(sd, cfg, &mut config)?;
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
        || original.env.iter().any(|(k, v)| config.env.get(k) != Some(v));
    if differs {
        return Err(OpenDanError::RuntimeMismatch {
            bound: "prompt.llm_context.runtime of the session".into(),
            provided: format!("{} {}", original.kind(), original.id.clone().unwrap_or_default()),
        });
    }
    if config == original {
        return Ok(provided.clone());
    }
    if config.kind() == REMOTE_NODE_KIND {
        return Err(OpenDanError::RuntimeMismatch {
            bound: "the session's frozen node runtime".into(),
            provided: format!("{} with another environment", original.kind()),
        });
    }
    agent_tool::runtime::RuntimeRegistry::from_config(&config)
        .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// The schema of `binding.json`: version 5 binds the session-runtime and
/// the session's working directory there; the workspace is recorded beside
/// it, never as the working directory.
pub const BINDING_SCHEMA: &str = "opendan.binding/5";

pub async fn bind_or_verify(
    sd: &SessionDir,
    lease: &crate::lock::Lease,
    rt: &dyn AgentRuntime,
    cfg: &SessionConfig,
    agent: &dyn crate::state::AgentStateClient,
    app_tools: &[String],
    plan: &BinPlan,
    runner_id: &str,
) -> Result<Binding> {
    let agent_root = agent.agent_root();
    if rt.config().kind() == "remote_ssh" {
        return Err(OpenDanError::Bind("Capability: remote Session helpers are not deployed; use SSH runtime with xllm independently".into()));
    }
    let node = cfg.runtime.is_node();
    let fallback = resolve_workdir(sd)?;
    let open_workdir = if node {
        PathBuf::from(rt.config().workdir.ok_or_else(|| {
            OpenDanError::Bind("the node session has no working directory yet".into())
        })?)
    } else {
        fallback.clone()
    };
    let open = RuntimeOpenCtx::new(&open_workdir, sd.sid(), LoopModel::FunctionCall);
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
    let existing = sd.binding_opt()?;
    let binding = Binding {
        schema: BINDING_SCHEMA.into(),
        runtime_id: desc.runtime_id.clone(),
        kind: desc.kind.clone(),
        target: desc.target.clone(),
        workdir: desc.workdir.clone(),
        workspace: cfg.workspace_binding.clone(),
        bound_at_ms: crate::now_ms(),
        bound_by: runner_id.into(),
    };
    let workdir_matches = if node {
        binding.workdir == open_workdir.to_string_lossy()
    } else {
        Path::new(&binding.workdir).canonicalize().ok().as_ref() == Some(&fallback)
    };
    if !workdir_matches {
        return Err(OpenDanError::Bind(
            "the runtime's working directory differs from the session's".into(),
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
    let layers = if node { Vec::new() } else { session_layers(sd, cfg, agent_root) };
    let windows_target = node && desc.capabilities.os == "windows";
    let tools: Vec<&str> = cfg.runtime.requirement.tools.iter().map(String::as_str).collect();
    if !tools.is_empty() {
        let probe = crate::baseline::missing_probe(&tools, windows_target);
        let result = run_probe(rt, &open_workdir, layers, sd.sid(), &sd.state_dir().join("runtime_ops"), probe, 5000).await?;
        let missing = crate::baseline::parse_missing(&result.stdout, &tools);
        if result.exit_code != 0 || !missing.is_empty() {
            return Err(OpenDanError::Bind(format!(
                "runtime lacks required tool {}",
                if missing.is_empty() { tools.join(", ") } else { missing.join(", ") }
            )));
        }
    }
    if node {
        if let Err(e) = write_session_context(sd, cfg, rt, agent, &binding.workdir).await {
            log::warn!("session {}: no session context on runtime {}: {e}", sd.sid(), binding.runtime_id);
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

/// Write the session's context into its working directory on a node
/// runtime, with a token signed now ([`crate::session_context`]): how its
/// helpers there reach AgentService. Rewritten at every binding, so a lost
/// file or an old token is replaced by the next drive.
async fn write_session_context(
    sd: &SessionDir,
    cfg: &SessionConfig,
    rt: &dyn AgentRuntime,
    agent: &dyn crate::state::AgentStateClient,
    workdir: &str,
) -> Result<()> {
    let root = agent
        .agent_root()
        .ok_or_else(|| OpenDanError::Bind("no AgentRoot here to sign the session's token".into()))?;
    let key = crate::session_context::TokenKey::load_or_create(root)?;
    let state_url = crate::session_context::node_state_url(agent.agent_id());
    let (ctx, token) = crate::session_context::new_context(&key, agent.agent_id(), cfg, workdir, state_url);
    let target = open_node_target_of(cfg.runtime.runtime_id(), &rt.config(), &sd.state_dir().join("runtime_ops")).await?;
    crate::session_context::write_context(&target, &ctx, &token).await
}

/// Open `rt` at `workdir` without tools and run one command there (binding
/// probes, baseline checks).
pub async fn run_probe(
    rt: &dyn AgentRuntime,
    workdir: &Path,
    path_prefix: Vec<PathBuf>,
    session_id: &str,
    ops_dir: &Path,
    command: String,
    timeout_ms: u64,
) -> Result<agent_tool::llm_bash::BashRunOutput> {
    let mut open = RuntimeOpenCtx::new(workdir, session_id, LoopModel::FunctionCall);
    open.path_prefix = path_prefix;
    let (_, mut sandbox) = rt
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
    // A node keeps the operation's locator in a run directory.
    sandbox.bind_run_dir("probe", Some(ops_dir.to_path_buf()));
    agent_tool::runtime::Sandbox::exec(
        &sandbox,
        agent_tool::llm_bash::BashRunRequest {
            command,
            cwd: workdir.to_path_buf(),
            timeout_ms,
            max_output_bytes: 16 * 1024,
            env: Vec::new(),
            target: agent_tool::llm_bash::BashTarget::Local,
            // A node runs a command only for an identified call.
            call_id: Some(format!("probe-{}", uuid::Uuid::new_v4().simple())),
        },
        &agent_tool::SessionRuntimeContext {
            trace_id: session_id.into(),
            agent_name: "libopendan".into(),
            behavior: "runtime_probe".into(),
            tool_call_index: 0,
            wakeup_id: String::new(),
            session_id: session_id.into(),
            read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
        },
    )
    .await
    .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// The standard tools of a runtime ([`crate::baseline`]): looked for on this
/// machine for the agent's own container (helpers come from the session
/// bin), through the runtime's executor for a node. A check that cannot run
/// says why; it never changes the declared baseline.
pub async fn check_baseline(record: &RuntimeRecord, ops_dir: &Path) -> BaselineCheck {
    let checked = async {
        if record.kind == RuntimeKind::Container {
            let names = crate::baseline::required_commands(true);
            let beside: Vec<PathBuf> = std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .into_iter()
                .collect();
            return Ok(crate::baseline::missing_locally(&names, None, &beside));
        }
        let exec = record
            .exec
            .as_ref()
            .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` is not enabled", record.runtime_id)))?;
        let mut config = node_config_of(&record.runtime_id, exec)?;
        let root = record.roots.first().cloned().ok_or_else(|| {
            OpenDanError::Bind(format!("runtime `{}`: the node grants no working directory root", record.runtime_id))
        })?;
        config.id = Some(record.runtime_id.clone());
        config.workdir = Some(root.display().to_string());
        let deps = XllmDeps {
            node_executor: Some(node_connector()),
            ..Default::default()
        };
        let rt = agent_tool::runtime::RuntimeRegistry::from_config_with(&config, &deps)
            .map_err(|e| OpenDanError::Bind(e.to_string()))?;
        let names = crate::baseline::required_commands(false);
        let probe = crate::baseline::missing_probe(&names, record.platform.os == "windows");
        let out = run_probe(rt.as_ref(), &root, Vec::new(), "baseline-check", ops_dir, probe, 30_000).await?;
        if out.exit_code != 0 {
            return Err(OpenDanError::Bind(format!(
                "the check exited {}: {}",
                out.exit_code,
                out.stderr.trim()
            )));
        }
        Ok(crate::baseline::parse_missing(&out.stdout, &names))
    }
    .await;
    let checked_at_ms = crate::now_ms();
    match checked {
        Ok(missing) => BaselineCheck {
            checked_at_ms,
            ok: missing.is_empty(),
            missing,
            error: None,
        },
        Err(e) => BaselineCheck {
            checked_at_ms,
            ok: false,
            missing: Vec::new(),
            error: Some(e.to_string()),
        },
    }
}

/// The session's PATH layers live on this machine: a node runtime gets none
/// and finds its commands on the node's own PATH (TODO §6.1).
pub async fn open_session_env(binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv> {
    let mut layers = Vec::new();
    if binding.kind != REMOTE_NODE_KIND {
        layers.push(ctx.session_dir.join(RUNTIME_DIR).join("bin"));
        if let Some(root) = &ctx.agent_root {
            layers.extend(skill_tool_dirs(root, &ctx.cmd_skills));
            if root.join("tools").is_dir() {
                layers.push(root.join("tools"));
            }
        }
    }
    let env = session_env_vars(ctx, &binding.runtime_id).await?;
    Ok(SessionEnv {
        runtime_id: binding.runtime_id.clone(),
        kind: binding.kind.clone(),
        workdir: binding.workdir.clone().into(),
        path_layers: layers,
        env,
    })
}

/// PATH layers of a local session: the session bin, the frozen cmd_skills'
/// commands, the agent's tools.
fn session_layers(sd: &SessionDir, cfg: &SessionConfig, agent_root: Option<&Path>) -> Vec<PathBuf> {
    let mut layers = vec![sd.runtime_bin_dir()];
    if let Some(root) = agent_root {
        layers.extend(skill_tool_dirs(root, &frozen_cmd_skills(cfg)));
        if root.join("tools").is_dir() {
            layers.push(root.join("tools"));
        }
    }
    layers
}

pub fn bin_plan_for(
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    session_cli: Option<PathBuf>,
    runtime_layers: &[PathBuf],
) -> Result<BinPlan> {
    bin_plan_with(cfg, agent_root, session_cli, &[], runtime_layers)
}

/// [`bin_plan_for`] with the host's helper commands.
pub fn bin_plan_with(
    cfg: &SessionConfig,
    agent_root: Option<&Path>,
    session_cli: Option<PathBuf>,
    helpers: &[bin_overlay::SessionHelper],
    runtime_layers: &[PathBuf],
) -> Result<BinPlan> {
    let mut plan = BinPlan {
        session_cli,
        helpers: helpers.to_vec(),
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
    if let Some(w) = &ctx.workspace {
        v.push(("OPENDAN_WORKSPACE_ID".to_string(), w.workspace_id.clone()));
        v.push((
            "OPENDAN_WORKSPACE_RUNTIME".to_string(),
            canonical_runtime(&w.location.runtime_id).to_string(),
        ));
        v.push((
            "OPENDAN_WORKSPACE_DIR".to_string(),
            w.location.directory.display().to_string(),
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_exec_must_be_a_complete_remote_node_config() {
        let ok = node_exec("did:dev:abc", "did:web:jarvis.example.com");
        let config = node_config_of("host", &ok).unwrap();
        assert_eq!(config.kind(), REMOTE_NODE_KIND);
        assert!(node_config_of("host", &serde_json::json!({ "kind": "native" })).is_err());
        assert!(node_config_of("host", &serde_json::json!({ "kind": "remote_node" })).is_err());
    }

    #[test]
    fn target_paths_follow_the_target_os() {
        assert_eq!(target_join("/home/o", &[".opendan", "a"]), "/home/o/.opendan/a");
        assert_eq!(target_join(r"C:\Users\o\", &[".opendan", "a"]), r"C:\Users\o\.opendan\a");
    }
}
