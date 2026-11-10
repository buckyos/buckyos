pub mod bin_overlay;
pub use agent_tool::runtime::{native_host_id, AgentRuntime, NativeRuntime};
pub use bin_overlay::{BinPlan, ToolPlan};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use agent_tool::runtime::executor::{ExecutorDirectory, ExecutorTarget};
use agent_tool::runtime::files::{LocalFileBackend, RuntimeFiles, WholeWriteMode, WriteChunk};
use agent_tool::runtime::node::{NodeExecutorConnector, RemoteNodeTarget, REMOTE_NODE_KIND};
use agent_tool::runtime::{RuntimeConfig, RuntimeOpenCtx};
use agent_tool::xllm::{LoopModel, ToolsConfig, XllmDeps};
use async_trait::async_trait;
use crate::baseline::CommandOrigin;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

static NODE_CONNECTOR: RwLock<Option<Arc<dyn NodeExecutorConnector>>> = RwLock::new(None);
static EXECUTOR_RESOLVER: RwLock<Option<Arc<dyn ExecutorResolver>>> = RwLock::new(None);

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

/// How a standalone executor (form B, daemon URL) is reached: the route the
/// control panel hands out for it now.
pub struct ExecutorRoute {
    pub connector: Arc<dyn NodeExecutorConnector>,
    pub profile_id: String,
    pub expires_at_ms: Option<u64>,
}

/// Resolves a standalone executor's id to its connector (tests, hosts with
/// their own directory). Default: the control panel's online list.
#[async_trait]
pub trait ExecutorResolver: Send + Sync {
    async fn resolve(&self, executor_id: &str, profile_id: &str) -> Result<ExecutorRoute>;
}

pub fn set_executor_resolver(resolver: Arc<dyn ExecutorResolver>) {
    *EXECUTOR_RESOLVER.write().expect("executor resolver") = Some(resolver);
}

struct BuckyosExecutorResolver;

#[async_trait]
impl ExecutorResolver for BuckyosExecutorResolver {
    async fn resolve(&self, executor_id: &str, profile_id: &str) -> Result<ExecutorRoute> {
        let directory = ExecutorDirectory::from_buckyos_runtime();
        let online = directory.list().await.map_err(OpenDanError::Bind)?;
        let target = online.iter().find(|t| t.executor_id == executor_id).ok_or_else(|| {
            OpenDanError::Bind(format!("executor `{executor_id}` is not online (see `executor_list`)"))
        })?;
        let profile = if profile_id.is_empty() {
            match target.profiles.as_slice() {
                [one] => one.clone(),
                [] => return Err(OpenDanError::Bind(format!("executor `{executor_id}` offers no profile to this agent"))),
                many => {
                    return Err(OpenDanError::Bind(format!(
                        "executor `{executor_id}` offers several profiles ({}); the runtime setting must name one",
                        many.join(", ")
                    )))
                }
            }
        } else {
            if !target.profiles.iter().any(|p| p == profile_id) {
                return Err(OpenDanError::Bind(format!("executor `{executor_id}` does not offer profile `{profile_id}` now")));
            }
            profile_id.to_string()
        };
        let connector = directory
            .connector(ExecutorTarget {
                executor_id: executor_id.to_string(),
                generation: target.generation,
                profile_id: profile.clone(),
                store_id: target.store_id.clone(),
            })
            .await
            .map_err(OpenDanError::Bind)?;
        Ok(ExecutorRoute { connector, profile_id: profile, expires_at_ms: target.expires_at_ms })
    }
}

fn executor_resolver() -> Arc<dyn ExecutorResolver> {
    EXECUTOR_RESOLVER
        .read()
        .expect("executor resolver")
        .clone()
        .unwrap_or_else(|| Arc::new(BuckyosExecutorResolver))
}

/// The `remote_node` configuration a node runtime's record carries.
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
    let executor = node.and_then(|n| n.executor_id.as_deref()).is_some_and(|e| !e.is_empty());
    if node.and_then(|n| n.connect_target()).is_none() {
        return Err(OpenDanError::Bind(format!(
            "runtime `{runtime_id}`: remote_node needs node_did or executor_id"
        )));
    }
    if !executor && node.and_then(|n| n.profile_id.as_deref()).is_none_or(str::is_empty) {
        return Err(OpenDanError::Bind(format!(
            "runtime `{runtime_id}`: remote_node needs node_did and profile_id"
        )));
    }
    Ok(config)
}

/// The connector a node runtime configuration executes through, and the
/// configuration with the executor's profile resolved; `expires_at_ms`
/// when a standalone executor's route has a term.
pub async fn connector_for(
    runtime_id: &str,
    config: &RuntimeConfig,
) -> Result<(Arc<dyn NodeExecutorConnector>, RuntimeConfig, Option<u64>)> {
    let mut config = config.clone();
    let node = config.remote_node.clone().unwrap_or_default();
    match node.executor_id.as_deref().filter(|e| !e.is_empty()) {
        Some(executor_id) => {
            let route = executor_resolver()
                .resolve(executor_id, node.profile_id.as_deref().unwrap_or_default())
                .await
                .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: {e}")))?;
            if let Some(n) = config.remote_node.as_mut() {
                n.profile_id = Some(route.profile_id);
            }
            Ok((route.connector, config, route.expires_at_ms))
        }
        None => Ok((node_connector(), config, None)),
    }
}

/// A node runtime opened as a target directory, with what its route said.
pub struct NodeTarget {
    target: RemoteNodeTarget,
    expires_at_ms: Option<u64>,
}

impl NodeTarget {
    /// When a standalone executor's grant ends, as its route reports.
    pub fn expires_at_ms(&self) -> Option<u64> {
        self.expires_at_ms
    }

    pub fn into_files(self) -> Box<dyn RuntimeFiles> {
        Box::new(self.target)
    }

    pub fn into_inner(self) -> RemoteNodeTarget {
        self.target
    }
}

impl std::ops::Deref for NodeTarget {
    type Target = RemoteNodeTarget;
    fn deref(&self) -> &RemoteNodeTarget {
        &self.target
    }
}

/// Open a node runtime as a target directory (workspace placement, the
/// default session directory): login + describe through its connector.
pub async fn open_node_target(record: &RuntimeRecord, ops_dir: &Path) -> Result<NodeTarget> {
    let exec = record
        .exec()
        .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` is not enabled", record.runtime_id)))?;
    let config = node_config_of(&record.runtime_id, &exec)?;
    open_node_target_of(&record.runtime_id, &config, ops_dir).await
}

pub async fn open_node_target_of(runtime_id: &str, config: &RuntimeConfig, ops_dir: &Path) -> Result<NodeTarget> {
    let (connector, config, expires_at_ms) = connector_for(runtime_id, config).await?;
    let node = config.remote_node.clone().unwrap_or_default();
    let target = RemoteNodeTarget::open(
        connector,
        node.connect_target().unwrap_or_default(),
        node.profile_id.as_deref().unwrap_or_default(),
        runtime_id,
        ops_dir.to_path_buf(),
    )
    .await
    .map_err(|e| OpenDanError::Bind(format!("runtime `{runtime_id}`: {e}")))?;
    Ok(NodeTarget { target, expires_at_ms })
}

const NODE_SESSIONS_DIR: &str = "sessions";

/// The directory of this agent on a node: `<account home>/.opendan/<agent_id>`
/// (directory convention), or `<first granted root>/.opendan/<agent_id>`
/// when the account home lies outside the granted roots (a device issued
/// for one work directory).
pub fn node_agent_dir(target: &RemoteNodeTarget, runtime_id: &str, agent_id: &str) -> Result<String> {
    let home = target.described().info.home.clone().filter(|h| !h.trim().is_empty());
    let roots = target.workdir_roots();
    let grant = RuntimeGrant { roots: roots.iter().map(PathBuf::from).collect(), ..Default::default() };
    let base = match home {
        Some(home) if grant.within_roots(&home) => home,
        _ => roots.first().cloned().ok_or_else(|| {
            OpenDanError::Bind(format!(
                "runtime `{runtime_id}`: its executor reports no account home and grants no directory, so the agent's directory there cannot be placed"
            ))
        })?,
    };
    Ok(target_join(&base, &[".opendan", agent_id]))
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
    /// The session's workspace: where it is, beside the working directory.
    pub workspace: Option<WorkspaceBinding>,
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
    let native = is_private_runtime(&record.location.runtime_id);
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
            // The binding may be narrower than the grant (a read-only plan).
            let narrowed = reference.access == WorkspaceAccess::ReadWrite
                && binding.access == WorkspaceAccess::ReadOnly;
            if reference.workspace_id != binding.workspace_id
                || (reference.access != binding.access && !narrowed)
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
            if is_private_runtime(&record.location.runtime_id) {
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
    if state.run_state != RunState::Waiting
        || !state.watched_tasks.is_empty()
        || state.wait_children.is_empty()
    {
        return Ok(Vec::new());
    }
    let mut children = Vec::new();
    for reference in &state.wait_children {
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
        config.baseline = Some(frozen.baseline());
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
    if let Some(frozen) = cfg.runtime.frozen.as_ref().filter(|f| f.is_node()) {
        let mut config = node_config_of(&frozen.runtime_id, &frozen.exec())?;
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
        config.baseline = Some(frozen.baseline());
        config.workdir = Some(node_workdir(sd, agent, &frozen.runtime_id, &config).await?);
        let mut deps = deps.clone();
        let device = config.remote_node.as_ref().and_then(|n| n.executor_id.as_deref()).is_some_and(|e| !e.is_empty());
        if deps.node_executor.is_none() || device {
            let (connector, resolved, _) = connector_for(&frozen.runtime_id, &config).await?;
            deps.node_executor = Some(connector);
            config = resolved;
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
    if config.kind() == "native" {
        if let Some(record) = agent.runtimes().lookup(PRIVATE_RUNTIME).await? {
            if let RuntimeConnector::Native { computer, .. } = record.connector {
                config.computer = computer;
            }
        }
    }
    agent_tool::runtime::RuntimeRegistry::from_config(&config)
        .map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// Where a node-runtime session works on its node: what is bound, else
/// `<account home>/.opendan/<agent>/sessions/<sid>` ([`node_agent_dir`]),
/// created now (the executor's `mkdir`, idempotent).
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
    let base = node_agent_dir(&target, runtime_id, agent.agent_id())?;
    let dir = target_join(&base, &[NODE_SESSIONS_DIR, sd.sid()]);
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

/// One runtime as a prompt sees it (the plan table's columns, runtime
/// lifecycle TODO §5.3).
pub fn runtime_entry(r: &RuntimeRecord) -> serde_json::Value {
    let mut v = serde_json::json!({
        "id": r.runtime_id,
        "name": r.name,
        "connector": r.connector.as_str(),
        "os": r.caps.platform.os,
        "shell": r.caps.platform.shell,
        "baseline": r.caps.baseline.as_str(),
        "available": r.usable(),
        "default": r.default,
        "private": r.is_private(),
        "computer": r.grant.computer.as_str(),
        "read_only": r.grant.read_only(),
        "workspace": r.grant.no_workspace().is_none(),
        "roots": r.grant.roots,
        "desktop": r.caps.desktop,
        "desktop_error": r.caps.desktop_error,
        "desc": r.desc,
    });
    if let Some(t) = &r.caps.tools_check {
        v["tools"] = serde_json::json!(t.present);
    }
    if let Some(ext) = r.caps.baseline_check.as_ref().and_then(|c| c.ext.as_ref()) {
        v["ext"] = serde_json::json!(ext.summary());
    }
    if let Some(t) = r.grant.expires_at_ms {
        v["expires_at_ms"] = serde_json::json!(t);
    }
    if !r.usable() {
        v["error"] = serde_json::json!(r.last_error);
    }
    v
}

/// One readable line of a runtime record (`runtime list`, the plan's
/// `<runtimes>` table).
pub fn runtime_line(r: &RuntimeRecord) -> String {
    let one_line = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let platform = [&r.caps.platform.os, &r.caps.platform.arch, &r.caps.platform.shell]
        .into_iter()
        .filter(|s| !s.trim().is_empty())
        .map(|s| one_line(s))
        .collect::<Vec<_>>();
    let mut flags = Vec::new();
    if r.is_private() {
        flags.push("private".to_string());
    }
    if r.default {
        flags.push("owner default".to_string());
    }
    if r.lifecycle == RuntimeLifecycle::Retired {
        flags.push("retired".to_string());
    } else if !r.enabled {
        flags.push("disabled".to_string());
    }
    if r.grant.read_only() {
        flags.push("read-only".to_string());
    }
    if r.grant.no_workspace().is_some() {
        flags.push("no workspaces".to_string());
    }
    if r.caps.desktop {
        flags.push("desktop".to_string());
    }
    if let Some(t) = r.grant.expires_at_ms {
        flags.push(format!("expires {t}"));
    }
    if r.caps.baseline_check.as_ref().is_some_and(|c| !c.dev_helpers().is_empty()) {
        flags.push("dev helpers".to_string());
    }
    let availability = serde_json::to_value(r.availability)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut line = format!(
        "- {}: {} — {}; {}; {}; {}; computer: {}{}",
        r.runtime_id,
        one_line(&r.name),
        r.connector.as_str(),
        if platform.is_empty() { "platform unknown".into() } else { platform.join(" / ") },
        r.caps.baseline.as_str(),
        availability,
        r.grant.computer.as_str(),
        if flags.is_empty() { String::new() } else { format!(" [{}]", flags.join(", ")) },
    );
    if !r.grant.roots.is_empty() {
        line.push_str(&format!("; roots: {}", r.grant.roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")));
    }
    if let Some(ext) = r.caps.baseline_check.as_ref().and_then(|c| c.ext.as_ref()) {
        line.push_str(&format!("; ext: {}", ext.summary()));
    }
    if let Some(t) = r.caps.tools_check.as_ref().filter(|t| !t.present.is_empty()) {
        line.push_str(&format!("; commands: {}", t.present.join(", ")));
    }
    if !r.desc.is_empty() {
        line.push_str(&format!(" — {}", one_line(&r.desc)));
    }
    line
}

/// The runtimes a session's prompt knows (`runtime_status.runtimes`), by
/// its `runtime_view` (runtime lifecycle TODO §0.3): a chat its private
/// runtime and the owner's default (M2); work its working runtime and the
/// private one; a plan every active runtime, with one readable line each
/// for its `<runtimes>` table (M6).
pub async fn runtimes_view(agent: &dyn crate::state::AgentStateClient, cfg: &SessionConfig) -> serde_json::Value {
    let view = cfg.session.policy.runtime_view;
    let mut all: Vec<RuntimeRecord> = agent
        .runtimes()
        .list()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.lifecycle == RuntimeLifecycle::Active)
        .collect();
    let with_default = matches!(view, RuntimeView::Chat | RuntimeView::All);
    for shown in all.iter_mut().filter(|r| r.is_private() || (with_default && r.default)) {
        if let Ok(Some(checked)) = lookup_rechecked(agent, &shown.runtime_id).await {
            *shown = checked;
        }
    }
    let private = all.iter().find(|r| r.is_private()).map(runtime_entry);
    let owner_default = all.iter().find(|r| r.default).map(runtime_entry);
    let working = cfg.runtime.runtime_id();
    match view {
        RuntimeView::Chat => serde_json::json!({
            "view": "chat",
            "private": private,
            "owner_default": owner_default,
        }),
        RuntimeView::Work => serde_json::json!({
            "view": "work",
            "working": working,
            "private": private,
        }),
        RuntimeView::All => serde_json::json!({
            "view": "all",
            "working": working,
            "private": private,
            "owner_default": owner_default,
            "all": all.iter().map(runtime_entry).collect::<Vec<_>>(),
            "lines": all.iter().map(runtime_line).collect::<Vec<_>>(),
        }),
    }
}

/// The schema of `binding.json`: version 6 binds the working runtime (by
/// its connector) and the session's working directory there; the workspace
/// is recorded beside it, never as the working directory.
pub const BINDING_SCHEMA: &str = "opendan.binding/6";

/// The connector of a runtime descriptor's kind (`native`, `remote_node`).
pub fn connector_of_kind(kind: &str) -> &'static str {
    if kind == REMOTE_NODE_KIND { "node_executor" } else { "native" }
}

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
        connector: connector_of_kind(&desc.kind).into(),
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
                bound: format!("{} {} {} {}", b.runtime_id, b.connector, b.target, b.workdir),
                provided: format!(
                    "{} {} {} {}",
                    binding.runtime_id, binding.connector, binding.target, binding.workdir
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
    let layers = if node { Vec::new() } else { session_layers(sd, agent_root) };
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
    crate::session_context::write_context(&*target, &ctx, &token).await
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
    run_with_env(rt, workdir, path_prefix, session_id, ops_dir, command, timeout_ms, Vec::new()).await
}

#[allow(clippy::too_many_arguments)]
async fn run_with_env(
    rt: &dyn AgentRuntime,
    workdir: &Path,
    path_prefix: Vec<PathBuf>,
    session_id: &str,
    ops_dir: &Path,
    command: String,
    timeout_ms: u64,
    env: Vec<(String, String)>,
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
            env,
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

/// A node runtime record as an `AgentRuntime` working in `workdir`
/// (checks, agent-level operations).
async fn node_runtime_of(record: &RuntimeRecord, workdir: &str) -> Result<Arc<dyn AgentRuntime>> {
    let exec = record
        .exec()
        .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` is not enabled", record.runtime_id)))?;
    let mut config = node_config_of(&record.runtime_id, &exec)?;
    config.id = Some(record.runtime_id.clone());
    config.workdir = Some(workdir.to_string());
    config.baseline = Some(record.caps.baseline);
    let (connector, config, _) = connector_for(&record.runtime_id, &config).await?;
    let deps = XllmDeps {
        node_executor: Some(connector),
        ..Default::default()
    };
    agent_tool::runtime::RuntimeRegistry::from_config_with(&config, &deps).map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// This process's environment as an `AgentRuntime` working in `workdir`.
fn private_runtime_of(workdir: &Path) -> Result<Arc<dyn AgentRuntime>> {
    let config = RuntimeConfig {
        kind: Some("native".into()),
        id: Some(PRIVATE_RUNTIME.into()),
        workdir: Some(workdir.display().to_string()),
        ..Default::default()
    };
    agent_tool::runtime::RuntimeRegistry::from_config(&config).map_err(|e| OpenDanError::Bind(e.to_string()))
}

/// The first granted root of a node runtime: where checks run.
fn node_check_dir(record: &RuntimeRecord) -> Result<String> {
    record
        .grant
        .roots
        .first()
        .map(|p| p.display().to_string())
        .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}`: no granted root is known yet (check it first)", record.runtime_id)))
}

/// Where `names` are on `record`'s runtime and who provides them; `versions`
/// (a subset) are also run, so a stub tells itself apart. On this machine the
/// standard helpers may come from beside the program.
async fn resolve_on(record: &RuntimeRecord, ops_dir: &Path, names: &[&str], versions: &[&str], agent_root: Option<&Path>, what: &str) -> Result<BTreeMap<String, ResolvedCommand>> {
    let timeout_ms = if versions.is_empty() { 30_000 } else { CHECK_FULL_TIMEOUT_MS };
    let (origin, probed) = if record.is_private() {
        let beside: Vec<PathBuf> = crate::baseline::beside_current_exe().into_iter().collect();
        let mut agent_dirs = agent_root.map(crate::baseline::agent_tool_dirs).unwrap_or_default();
        agent_dirs.extend(crate::baseline::managed_tool_dirs());
        let origin = CommandOrigin {
            root: crate::baseline::buckyos_root().map(|r| r.display().to_string()),
            agent_root: agent_root.map(|r| r.display().to_string()),
            beside: beside.first().map(|d| d.display().to_string()),
            windows: cfg!(windows),
        };
        let mut probed: BTreeMap<String, crate::baseline::ProbedCommand> = crate::baseline::resolve_locally(names, &agent_dirs, None, &beside)
            .into_iter()
            .map(|(name, path)| (name, crate::baseline::ProbedCommand { path, exit_code: None, version: None }))
            .collect();
        if !versions.is_empty() {
            std::fs::create_dir_all(ops_dir).map_err(|e| OpenDanError::io(ops_dir, e))?;
            let rt = private_runtime_of(ops_dir)?;
            let probe = crate::baseline::resolve_probe(versions, versions, cfg!(windows));
            let out = run_probe(rt.as_ref(), ops_dir, agent_dirs, what, ops_dir, probe, timeout_ms).await?;
            if out.exit_code != 0 {
                return Err(OpenDanError::Bind(format!("the check exited {}: {}", out.exit_code, out.stderr.trim())));
            }
            for (name, ran) in crate::baseline::parse_resolved(&out.stdout, versions) {
                if let Some(found) = probed.get_mut(&name) {
                    found.exit_code = ran.exit_code;
                    found.version = ran.version;
                }
            }
        }
        (origin, probed)
    } else {
        let windows = record.caps.platform.os == "windows";
        let root = node_check_dir(record)?;
        let rt = node_runtime_of(record, &root).await?;
        let probe = crate::baseline::resolve_probe(names, versions, windows);
        let out = run_probe(rt.as_ref(), Path::new(&root), Vec::new(), what, ops_dir, probe, timeout_ms).await?;
        if out.exit_code != 0 {
            return Err(OpenDanError::Bind(format!("the check exited {}: {}", out.exit_code, out.stderr.trim())));
        }
        let origin = CommandOrigin { root: record.caps.buckyos_root.clone(), agent_root: None, beside: None, windows };
        (origin, crate::baseline::parse_resolved(&out.stdout, names))
    };
    Ok(probed
        .into_iter()
        .map(|(name, found)| {
            let stub = crate::baseline::is_stub_exit(found.exit_code);
            let command = ResolvedCommand {
                provided_by: origin.of(&found.path),
                path: found.path,
                version: if stub { None } else { found.version },
                stub,
            };
            (name, command)
        })
        .collect())
}

/// The tools of a runtime against its declared baseline ([`crate::baseline`]):
/// looked for on this machine for the agent's private runtime (helpers come
/// from the session bin), through the runtime's executor for a node. A
/// background check records where they are and keeps what a full one learned
/// of the same files; `full` also runs the extended ones, for their versions
/// and to find stubs. A check that cannot run says why; it never changes the
/// declared baseline.
pub async fn check_baseline(record: &RuntimeRecord, ops_dir: &Path, agent_root: Option<&Path>, full: bool) -> BaselineCheck {
    let overlay = record.is_private();
    let level = record.caps.baseline;
    let names = crate::baseline::checked_commands(overlay);
    let versions = if full { crate::baseline::extended_commands() } else { Vec::new() };
    let checked = resolve_on(record, ops_dir, &names, &versions, agent_root, "baseline-check").await;
    let checked_at_ms = crate::now_ms();
    match checked {
        Ok(mut resolved) => {
            if let Some(before) = record.caps.baseline_check.as_ref().filter(|_| !full) {
                crate::baseline::keep_versions(&mut resolved, &before.resolved);
            }
            let findings = crate::baseline::evaluate(level, overlay, &resolved);
            BaselineCheck {
                checked_at_ms,
                level,
                ok: findings.ok(),
                missing: findings.missing,
                unmanaged: findings.unmanaged,
                resolved,
                ext: Some(findings.ext),
                error: None,
            }
        }
        Err(e) => BaselineCheck {
            checked_at_ms,
            level,
            ok: false,
            missing: Vec::new(),
            unmanaged: Vec::new(),
            resolved: BTreeMap::new(),
            ext: None,
            error: Some(e.to_string()),
        },
    }
}

/// Whether `commands` exist on the runtime (`caps.tools_check`): skill
/// installation, `check --commands`. `full` also runs them, for their versions
/// and to find stubs; otherwise what a full check learned of the same files
/// stays.
pub async fn check_commands(record: &RuntimeRecord, ops_dir: &Path, commands: &[String], agent_root: Option<&Path>, full: bool) -> ToolsCheck {
    let names: Vec<&str> = commands.iter().map(String::as_str).collect();
    let versions = if full { names.clone() } else { Vec::new() };
    let checked = resolve_on(record, ops_dir, &names, &versions, agent_root, "tools-check").await;
    let checked_at_ms = crate::now_ms();
    match checked {
        Ok(mut resolved) => {
            if let Some(before) = record.caps.tools_check.as_ref().filter(|_| !full) {
                crate::baseline::keep_versions(&mut resolved, &before.resolved);
            }
            let (present, missing) = commands.iter().cloned().partition(|c| resolved.get(c).is_some_and(|r| !r.stub));
            ToolsCheck { checked_at_ms, present, missing, resolved, error: None }
        }
        Err(e) => ToolsCheck {
            checked_at_ms,
            present: Vec::new(),
            missing: Vec::new(),
            resolved: BTreeMap::new(),
            error: Some(e.to_string()),
        },
    }
}

pub async fn check_node_placement(agent: &dyn crate::state::AgentStateClient, record: &RuntimeRecord) -> Result<()> {
    let Ok(ops_dir) = runtime_ops_dir(agent) else {
        return Ok(());
    };
    let exec = record
        .exec()
        .ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` is not enabled", record.runtime_id)))?;
    let config = node_config_of(&record.runtime_id, &exec)?;
    let target = open_node_target_of(&record.runtime_id, &config, &ops_dir).await?;
    node_agent_dir(&target, &record.runtime_id, agent.agent_id())?;
    if !target.caps().contains(&buckyos_api::ExecutorOp::Mkdir) {
        return Err(OpenDanError::Bind(format!(
            "runtime `{}` does not allow creating directories, so a session's directory cannot be made there; use it from a private session (`agent-session run` / `copy`)",
            record.runtime_id
        )));
    }
    Ok(())
}

/// `<account home>/.opendan/<agent>` on the node of `record`.
async fn node_agent_dir_of(record: &RuntimeRecord, agent_id: &str, ops_dir: &Path) -> Option<String> {
    let config = node_config_of(&record.runtime_id, &record.exec()?).ok()?;
    let target = open_node_target_of(&record.runtime_id, &config, ops_dir).await.ok()?;
    node_agent_dir(&target, &record.runtime_id, agent_id).ok()
}

fn shell_quote(windows: bool, value: &str) -> String {
    if windows {
        format!("'{}'", value.replace('\'', "''"))
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// What is installed under a runtime's `tools/`, as the reconciler recorded it (`xagent runtime ext-tools status`), and with
/// `smoke` what each package's smoke says now (`check --full`). Read-only: nothing an Agent does installs anything.
pub async fn ext_tools_report(record: &RuntimeRecord, ops_dir: &Path, smoke: bool) -> Result<serde_json::Value> {
    use buckyos_api::ext_tools_install::{ExtToolsInstall, ToolsRoot};
    let mut smoke_results = serde_json::Map::new();
    let (root_text, managed_by, installed): (String, &str, BTreeMap<String, ExtToolsInstall>) = if record.is_private() {
        let root = crate::baseline::buckyos_root().ok_or_else(|| OpenDanError::Bind("cannot tell $BUCKYOS_ROOT".to_string()))?;
        let tools = ToolsRoot::new(root.join("tools"));
        let installed = tools.installed();
        if smoke {
            for id in installed.keys() {
                let root = ToolsRoot::new(tools.path());
                let package = id.clone();
                let result = tokio::task::spawn_blocking(move || root.check(&package)).await.map_err(|e| OpenDanError::Other(e.to_string()))?;
                smoke_results.insert(id.clone(), json_result(result));
            }
        }
        let in_container = std::env::var_os("BUCKYOS_EXTTOOL_DIR").is_some();
        (tools.path().display().to_string(), if in_container { "node_volume" } else { "node" }, installed)
    } else {
        let windows = record.caps.platform.os == "windows";
        let root = record.caps.buckyos_root.clone().ok_or_else(|| OpenDanError::Bind(format!("runtime `{}` has no BuckyOS root: it takes no extension packages", record.runtime_id)))?;
        let separator = if windows { "\\" } else { "/" };
        let tools = format!("{root}{separator}tools");
        let probe = if windows {
            format!("Get-ChildItem -Path {} -ErrorAction SilentlyContinue | ForEach-Object {{ Get-Content -Raw $_.FullName }}", shell_quote(true, &format!("{tools}\\installs\\*.json")))
        } else {
            format!("for f in {}/installs/*.json; do [ -f \"$f\" ] && cat \"$f\"; done; true", shell_quote(false, &tools))
        };
        let dir = node_check_dir(record)?;
        let rt = node_runtime_of(record, &dir).await?;
        let out = run_probe(rt.as_ref(), Path::new(&dir), Vec::new(), "ext-tools", ops_dir, probe, 30_000).await?;
        if out.exit_code != 0 {
            return Err(OpenDanError::Bind(format!("reading {tools}/installs exited {}: {}", out.exit_code, out.stderr.trim())));
        }
        let installed: BTreeMap<String, ExtToolsInstall> = serde_json::Deserializer::from_str(&out.stdout)
            .into_iter::<ExtToolsInstall>()
            .flatten()
            .map(|install| (install.id.clone(), install))
            .collect();
        if smoke {
            let program = if windows { format!("{root}\\bin\\opendan\\opendan.exe") } else { format!("{root}/bin/opendan/opendan") };
            for id in installed.keys() {
                let command = format!(
                    "{}{} ext-tools check --root {} --id {}",
                    if windows { "& " } else { "" },
                    shell_quote(windows, &program),
                    shell_quote(windows, &tools),
                    shell_quote(windows, id)
                );
                let out = run_probe(rt.as_ref(), Path::new(&dir), Vec::new(), "ext-tools", ops_dir, command, CHECK_FULL_TIMEOUT_MS).await?;
                let parsed: Option<serde_json::Value> = out.stdout.lines().rev().find_map(|line| serde_json::from_str(line).ok());
                let result = match parsed {
                    Some(v) if v["ok"] == true => Ok(()),
                    Some(v) => Err(v["error"].as_str().unwrap_or("the check failed").to_string()),
                    None => Err(format!("exit {}: {}", out.exit_code, out.stderr.trim())),
                };
                smoke_results.insert(id.clone(), json_result(result));
            }
        }
        (tools, "node", installed)
    };
    let mut report = serde_json::json!({ "runtime_id": record.runtime_id, "tools_root": root_text, "managed_by": managed_by, "packages": installed });
    if smoke {
        report["smoke"] = serde_json::Value::Object(smoke_results);
    }
    Ok(report)
}

fn json_result(result: std::result::Result<(), String>) -> serde_json::Value {
    match result {
        Ok(()) => serde_json::json!({ "ok": true }),
        Err(error) => serde_json::json!({ "ok": false, "error": error }),
    }
}

/// What one agent-level command on a runtime did (`runtime_exec`), kept
/// under `<agent_root>/state/runtime_ops/<op_id>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeOpRecord {
    pub op_id: String,
    pub runtime_id: String,
    pub command: String,
    pub cwd: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub timed_out: bool,
    pub started_at_ms: u64,
    pub finished_at_ms: u64,
    /// Who asked (a session id, `skill-install`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// The tool call a session ran it for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

/// A session's own record of an `agent-session run` (R-U12), under its
/// `state/runtime_ops/<op_id>.json`, beside the agent-level one.
pub fn session_run_record(sd: &SessionDir, op: &RuntimeOpRecord, call_id: Option<String>) -> Result<RuntimeOpRecord> {
    let mut record = op.clone();
    record.call_id = call_id;
    let dir = sd.state_dir().join(RUNTIME_OPS_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
    crate::fsutil::atomic_replace_json(&dir.join(format!("{}.json", record.op_id)), &record)?;
    Ok(record)
}

pub const RUNTIME_OPS_DIR: &str = "runtime_ops";
/// A full check runs every extended tool once.
const CHECK_FULL_TIMEOUT_MS: u64 = 120_000;
pub const RUNTIME_EXEC_DEFAULT_TIMEOUT_MS: u64 = 120_000;
pub const RUNTIME_EXEC_MAX_TIMEOUT_MS: u64 = 30 * 60 * 1000;

fn runtime_ops_dir(agent: &dyn crate::state::AgentStateClient) -> Result<PathBuf> {
    let root = agent
        .agent_root()
        .ok_or_else(|| OpenDanError::Bind("agent-level runtime operations need the AgentRoot on this host".into()))?;
    Ok(root.join("state").join(RUNTIME_OPS_DIR))
}

/// A runtime's record for something about to use it. A node runtime found
/// unreachable earlier, or a desktop found unavailable, is asked again
/// first: its executor may have come up or received its authorization
/// since (node_daemon opens it after loading the node's settings, the agent
/// may have checked first).
pub async fn lookup_rechecked(agent: &dyn crate::state::AgentStateClient, runtime_id: &str) -> Result<Option<RuntimeRecord>> {
    let record = agent.runtimes().lookup(runtime_id).await?;
    let Some(r) = record.as_ref().filter(|r| {
        ((r.availability != RuntimeAvailability::Available && r.connector.is_node()) || r.caps.desktop_error.is_some())
            && r.lifecycle == RuntimeLifecycle::Active
            && r.enabled
            && r.checked_at_ms.is_none_or(|t| crate::now_ms().saturating_sub(t) >= crate::api::RUNTIME_RECHECK_MS)
    }) else {
        return Ok(record);
    };
    match agent.runtimes().check(&r.runtime_id, &[], false).await {
        Ok(checked) => Ok(Some(checked)),
        Err(e) => {
            log::warn!("runtime {runtime_id} not checked again: {e}");
            Ok(record)
        }
    }
}

/// A runtime record usable for an agent-level operation now, else why not
/// (R-U5: refused, never replaced).
pub async fn usable_runtime(agent: &dyn crate::state::AgentStateClient, runtime_id: &str) -> Result<RuntimeRecord> {
    let record = lookup_rechecked(agent, runtime_id)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("runtime {runtime_id} (see `agent-session runtime list`)")))?;
    if record.lifecycle == RuntimeLifecycle::Retired {
        return Err(OpenDanError::Bind(format!("runtime `{}` is retired: it is kept for its history only", record.runtime_id)));
    }
    if !record.usable() {
        return Err(OpenDanError::Bind(format!(
            "runtime `{}` is not available: {}{}",
            record.runtime_id,
            record.last_error.clone().unwrap_or_else(|| format!("{:?}", record.availability)),
            match record.checked_at_ms {
                Some(t) => format!(" (last checked at {t}; an owner can open a runtime in the directory)"),
                None => String::new(),
            }
        )));
    }
    Ok(record)
}

/// Run one command on a runtime of the agent, outside any session (R-U3):
/// `agent-session run`, skill installation, tool checks. `cwd` must be
/// inside the runtime's granted roots (a node needs one; the private
/// runtime defaults to the AgentRoot). Output and exit code are returned
/// and archived under `state/runtime_ops/<op_id>.json`; a command that
/// outlives `timeout_ms` is reported as timed out with what it printed.
pub async fn runtime_exec(
    agent: &dyn crate::state::AgentStateClient,
    runtime_id: &str,
    command: &str,
    cwd: Option<&str>,
    timeout_ms: u64,
    by: Option<&str>,
) -> Result<RuntimeOpRecord> {
    let record = usable_runtime(agent, runtime_id).await?;
    if !record.grant.can_exec() {
        return Err(OpenDanError::Bind(format!(
            "runtime `{}` grants no Exec to this agent (ops: {:?})",
            record.runtime_id, record.grant.ops
        )));
    }
    let ops_dir = runtime_ops_dir(agent)?;
    let op_id = format!("op-{}", uuid::Uuid::new_v4().simple());
    let timeout_ms = timeout_ms.clamp(1_000, RUNTIME_EXEC_MAX_TIMEOUT_MS);
    let started_at_ms = crate::now_ms();
    let on_node = !record.is_private();
    let (rt, workdir, layers): (Arc<dyn AgentRuntime>, String, Vec<PathBuf>) = if record.is_private() {
        let dir = match cwd {
            Some(c) => PathBuf::from(c),
            None => agent.agent_root().map(Path::to_path_buf).unwrap_or_else(std::env::temp_dir),
        };
        if !dir.is_absolute() || !dir.is_dir() {
            return Err(OpenDanError::InvalidArgument(format!("--cwd {} is not an existing absolute directory on the private runtime", dir.display())));
        }
        (private_runtime_of(&dir)?, dir.display().to_string(), crate::baseline::managed_tool_dirs())
    } else {
        let dir = cwd
            .filter(|c| !c.trim().is_empty())
            .ok_or_else(|| OpenDanError::InvalidArgument(format!("--cwd is required on runtime `{}`: a directory inside its granted roots", record.runtime_id)))?;
        if !buckyos_api::is_absolute_node_path(dir) || dir.split(['/', '\\']).any(|p| p == "..") {
            return Err(OpenDanError::InvalidArgument(format!("--cwd `{dir}` must be an absolute path without `..`")));
        }
        if !record.grant.within_roots(dir) {
            return Err(OpenDanError::Bind(format!(
                "--cwd `{dir}` is outside the roots granted on `{}`: {}",
                record.runtime_id,
                record.grant.roots.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ")
            )));
        }
        (node_runtime_of(&record, dir).await?, dir.to_string(), Vec::new())
    };
    let env = if on_node {
        node_agent_dir_of(&record, agent.agent_id(), &ops_dir).await.map(|dir| crate::baseline::tool_cache_env(&Path::new(&dir).join("cache"))).unwrap_or_default()
    } else {
        agent_cache_env(false, &workdir, agent.agent_root())
    };
    let out = run_with_env(rt.as_ref(), Path::new(&workdir), layers, &op_id, &ops_dir, command.to_string(), timeout_ms, env).await;
    let finished_at_ms = crate::now_ms();
    let op = match out {
        Ok(out) => RuntimeOpRecord {
            op_id,
            runtime_id: record.runtime_id.clone(),
            command: command.to_string(),
            cwd: workdir,
            exit_code: out.exit_code,
            stdout: out.stdout,
            stderr: out.stderr,
            timed_out: out.timed_out,
            started_at_ms,
            finished_at_ms,
            by: by.map(str::to_string),
            call_id: None,
        },
        Err(e) => RuntimeOpRecord {
            op_id,
            runtime_id: record.runtime_id.clone(),
            command: command.to_string(),
            cwd: workdir,
            exit_code: 125,
            stdout: String::new(),
            stderr: e.to_string(),
            timed_out: false,
            started_at_ms,
            finished_at_ms,
            by: by.map(str::to_string),
            call_id: None,
        },
    };
    std::fs::create_dir_all(&ops_dir).map_err(|e| OpenDanError::io(&ops_dir, e))?;
    crate::fsutil::atomic_replace_json(&ops_dir.join(format!("{}.json", op.op_id)), &op)?;
    Ok(op)
}

/// The record of an earlier `runtime_exec` (a session recovering asks
/// instead of replaying).
pub fn runtime_op(agent: &dyn crate::state::AgentStateClient, op_id: &str) -> Result<Option<RuntimeOpRecord>> {
    crate::ids::validate_session_id(op_id).map_err(|_| OpenDanError::InvalidArgument(format!("bad op id `{op_id}`")))?;
    crate::fsutil::read_json_opt(&runtime_ops_dir(agent)?.join(format!("{op_id}.json")))
}

/// Put a local file or directory onto a runtime of the agent at
/// `remote_dir` (R-U3): skill commands, ExtTools installers. Files are
/// written whole in chunks; returns the remote paths written.
pub async fn runtime_put(
    agent: &dyn crate::state::AgentStateClient,
    runtime_id: &str,
    local: &Path,
    remote_dir: &str,
) -> Result<Vec<String>> {
    let record = usable_runtime(agent, runtime_id).await?;
    if !local.exists() {
        return Err(OpenDanError::InvalidArgument(format!("{} does not exist", local.display())));
    }
    if !buckyos_api::is_absolute_node_path(remote_dir) || remote_dir.split(['/', '\\']).any(|p| p == "..") {
        return Err(OpenDanError::InvalidArgument(format!("remote directory `{remote_dir}` must be an absolute path without `..`")));
    }
    if !record.grant.within_roots(remote_dir) {
        return Err(OpenDanError::Bind(format!("`{remote_dir}` is outside the roots granted on `{}`", record.runtime_id)));
    }
    let files: Box<dyn RuntimeFiles> = if record.is_private() {
        Box::new(LocalFileBackend)
    } else {
        open_node_target(&record, &runtime_ops_dir(agent)?).await?.into_files()
    };
    let mut written = Vec::new();
    let mut pending = vec![(local.to_path_buf(), remote_dir.to_string())];
    while let Some((path, dir)) = pending.pop() {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default().to_string();
        if path.is_dir() {
            let sub = if path == local { dir.clone() } else { target_join(&dir, &[&name]) };
            let rd = std::fs::read_dir(&path).map_err(|e| OpenDanError::io(&path, e))?;
            for entry in rd {
                let entry = entry.map_err(|e| OpenDanError::io(&path, e))?;
                pending.push((entry.path(), sub.clone()));
            }
            continue;
        }
        let data = std::fs::read(&path).map_err(|e| OpenDanError::io(&path, e))?;
        if data.len() as u64 > crate::transfer::MAX_NODE_TRANSFER_BYTES {
            return Err(OpenDanError::InvalidArgument(format!("{} is larger than {} bytes", path.display(), crate::transfer::MAX_NODE_TRANSFER_BYTES)));
        }
        let remote = target_join(&dir, &[&name]);
        let chunk = files.limits().max_chunk_bytes.max(1) as usize;
        let mut offset = 0usize;
        loop {
            let end = (offset + chunk).min(data.len());
            let (mode, expected_size) = if offset == 0 { (WholeWriteMode::Replace, None) } else { (WholeWriteMode::Append, Some(offset as u64)) };
            files
                .write_chunk(&remote, WriteChunk { mode, expected_size, expected_hash: None, data: &data[offset..end] })
                .await
                .map_err(|e| OpenDanError::Bind(format!("write {remote}: {:?}: {}", e.code, e.message)))?;
            offset = end;
            if offset >= data.len() {
                break;
            }
        }
        written.push(remote);
    }
    written.sort();
    Ok(written)
}

/// Tool caches of a session (ExtTools TODO §3.6). On a node they go next to the sessions in the Agent's directory there. On
/// the machine of the Agent Service they go under its AgentRoot, unless the process already has them: a worker's entrypoint
/// points them at the instance volume.
fn agent_cache_env(on_node: bool, workdir: &str, agent_root: Option<&Path>) -> Vec<(String, String)> {
    if on_node {
        // A node's path, which may be another OS's.
        let cut = [format!("/{NODE_SESSIONS_DIR}/"), format!("\\{NODE_SESSIONS_DIR}\\")].iter().filter_map(|marker| workdir.rfind(marker.as_str())).max();
        return cut.map(|at| crate::baseline::tool_cache_env(&Path::new(&workdir[..at]).join("cache"))).unwrap_or_default();
    }
    let Some(root) = agent_root else {
        return Vec::new();
    };
    crate::baseline::tool_cache_env(&root.join(".cache")).into_iter().filter(|(name, _)| std::env::var_os(name).is_none()).collect()
}

/// The session's PATH layers live on this machine: a node runtime gets none
/// and finds its commands on the node's own PATH (TODO §6.1).
pub async fn open_session_env(binding: &Binding, ctx: &SessionEnvCtx) -> Result<SessionEnv> {
    let mut layers = Vec::new();
    if binding.connector != "node_executor" {
        layers.push(ctx.session_dir.join(RUNTIME_DIR).join("bin"));
        if let Some(root) = &ctx.agent_root {
            if root.join("tools").is_dir() {
                layers.push(root.join("tools"));
            }
        }
        layers.extend(crate::baseline::managed_tool_dirs());
    }
    let mut env = session_env_vars(ctx, &binding.runtime_id).await?;
    env.extend(agent_cache_env(binding.connector == "node_executor", &binding.workdir, ctx.agent_root.as_deref()));
    Ok(SessionEnv {
        runtime_id: binding.runtime_id.clone(),
        kind: binding.connector.clone(),
        workdir: binding.workdir.clone().into(),
        path_layers: layers,
        env,
    })
}

/// PATH layers of a local session: the session bin, the agent's tools.
fn session_layers(sd: &SessionDir, agent_root: Option<&Path>) -> Vec<PathBuf> {
    let mut layers = vec![sd.runtime_bin_dir()];
    if let Some(root) = agent_root {
        if root.join("tools").is_dir() {
            layers.push(root.join("tools"));
        }
    }
    layers.extend(crate::baseline::managed_tool_dirs());
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
    fn caches_of_a_node_session_go_next_to_its_sessions() {
        let env = agent_cache_env(true, "/home/alice/.opendan/jarvis/sessions/s1", None);
        assert!(env.iter().any(|(k, v)| k == "DENO_DIR" && v == "/home/alice/.opendan/jarvis/cache/deno-cache"), "{env:?}");
        assert!(agent_cache_env(true, "/somewhere/else", None).is_empty(), "no agent directory, no opinion");
        let windows = agent_cache_env(true, "C:\\Users\\a\\.opendan\\j\\sessions\\s1", None);
        assert!(windows.iter().any(|(k, v)| k == "UV_CACHE_DIR" && v.starts_with("C:\\Users\\a\\.opendan\\j")), "{windows:?}");
    }

    #[test]
    fn node_exec_must_be_a_complete_remote_node_config() {
        let ok = RuntimeConnector::node("did:dev:abc", "did:web:jarvis.example.com").exec();
        let config = node_config_of("host", &ok).unwrap();
        assert_eq!(config.kind(), REMOTE_NODE_KIND);
        assert!(node_config_of("host", &serde_json::json!({ "kind": "native" })).is_err());
        assert!(node_config_of("host", &serde_json::json!({ "kind": "remote_node" })).is_err());
        // A standalone executor resolves its profile when it is reached.
        let exec = RuntimeConnector::executor("dev-1", None).exec();
        assert_eq!(node_config_of("exec-dev-1", &exec).unwrap().remote_node.unwrap().connect_target(), Some("dev-1"));
    }

    #[test]
    fn target_paths_follow_the_target_os() {
        assert_eq!(target_join("/home/o", &[".opendan", "a"]), "/home/o/.opendan/a");
        assert_eq!(target_join(r"C:\Users\o\", &[".opendan", "a"]), r"C:\Users\o\.opendan\a");
    }
}
