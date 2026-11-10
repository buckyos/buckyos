//! Runtimes of an agent (`state/runtimes/<runtime_id>.json`): where its
//! sessions and workspaces can run. A record says how a runtime is reached
//! (`connector`), what the machine can do (`caps`), what this agent may do
//! there (`grant`) and how the agent has used it (`usage`); executing in it
//! is the runtime layer's job (`agent_tool::runtime`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use agent_tool::runtime::{Capabilities, RuntimeDescriptor};
use buckyos_api::{
    owner_agent_profile_id, AgentRuntimeTarget, AgentSettings, ComputerAccess, ExecutorOp, InstallGrant,
    RuntimeBaseline,
};

pub const RUNTIME_RECORD_VERSION: u32 = 4;

/// The agent's private runtime: the environment its Agent Service runs in
/// (its aios container, or the host process when the service runs
/// natively). Always registered, always usable, declared `extended` in the
/// container; `$BUCKYOS_ROOT` has its full layout there.
pub const PRIVATE_RUNTIME: &str = "private";
/// The owner's desktop node (an id of the owner's settings).
pub const HOST_RUNTIME: &str = "host";
/// Former names of [`PRIVATE_RUNTIME`] (runtime records, workspace
/// locations, the owner's `default_runtime_id`), still read.
pub const LEGACY_CONTAINER_RUNTIME: &str = "container";
pub const LEGACY_LOCAL_RUNTIME: &str = "local";

/// The runtime an id names, aliases resolved.
pub fn canonical_runtime(runtime_id: &str) -> &str {
    match runtime_id {
        LEGACY_CONTAINER_RUNTIME | LEGACY_LOCAL_RUNTIME => PRIVATE_RUNTIME,
        other => other,
    }
}

/// Runs in this process's own environment (no executor in between).
pub fn is_private_runtime(runtime_id: &str) -> bool {
    canonical_runtime(runtime_id) == PRIVATE_RUNTIME
}

/// How a runtime is reached: the only type tag of a record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuntimeConnector {
    /// This process's own environment.
    Native {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        aliases: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        computer: Option<agent_tool::runtime::computer::NativeComputerConfig>,
    },
    /// A Node Executor: a node's embedded one (`node_did`) or a standalone
    /// executor (`executor_id`, form B), logged in under `profile_id`.
    NodeExecutor {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        node_did: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        executor_id: Option<String>,
        profile_id: String,
    },
    Ssh {
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        user: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity_file: Option<String>,
    },
}

impl RuntimeConnector {
    /// The owner-enabled node runtime: the node and the profile the
    /// scheduler derives for the agent there.
    pub fn node(node_did: &str, agent_did: &str) -> Self {
        RuntimeConnector::NodeExecutor {
            node_did: Some(node_did.to_string()),
            executor_id: None,
            profile_id: owner_agent_profile_id(agent_did),
        }
    }

    /// A standalone executor (daemon URL); an empty `profile_id` is
    /// resolved to the executor's only profile when it is reached.
    pub fn executor(executor_id: &str, profile_id: Option<&str>) -> Self {
        RuntimeConnector::NodeExecutor {
            node_did: None,
            executor_id: Some(executor_id.to_string()),
            profile_id: profile_id.unwrap_or_default().to_string(),
        }
    }

    pub fn is_native(&self) -> bool {
        matches!(self, RuntimeConnector::Native { .. })
    }

    /// Executed through a Node Executor (`remote_node` runtime config).
    pub fn is_node(&self) -> bool {
        matches!(self, RuntimeConnector::NodeExecutor { .. })
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeConnector::Native { .. } => "native",
            RuntimeConnector::NodeExecutor { .. } => "node_executor",
            RuntimeConnector::Ssh { .. } => "ssh",
        }
    }

    /// The id a standalone executor connector routes by.
    pub fn executor_id(&self) -> Option<&str> {
        match self {
            RuntimeConnector::NodeExecutor { executor_id: Some(id), .. } => Some(id),
            _ => None,
        }
    }

    /// The `agent_tool` RuntimeConfig this connector executes through.
    pub fn exec(&self) -> Value {
        match self {
            RuntimeConnector::Native { computer, .. } => {
                let mut config = serde_json::json!({ "kind": "native" });
                if let Some(computer) = computer {
                    config["computer"] = serde_json::json!(computer);
                }
                config
            },
            RuntimeConnector::NodeExecutor { node_did, executor_id, profile_id } => serde_json::json!({
                "kind": agent_tool::runtime::node::REMOTE_NODE_KIND,
                "remote_node": { "node_did": node_did, "executor_id": executor_id, "profile_id": profile_id },
            }),
            RuntimeConnector::Ssh { host, user, port, identity_file } => serde_json::json!({
                "kind": "remote_ssh",
                "remote_ssh": { "host": host, "user": user, "port": port, "identity_file": identity_file },
            }),
        }
    }
}

/// The platform of a runtime: `os` is `linux | macos | windows`, `shell`
/// the program name commands are written for (`bash`, `pwsh`, …).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimePlatform {
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub shell: String,
}

impl RuntimePlatform {
    /// The platform as a runtime reports it (`darwin` is `macos`; the shell
    /// by its program name).
    pub fn reported(os: &str, arch: &str, shell: &str) -> Self {
        let info = agent_tool::runtime::RuntimeInfo {
            os: os.to_string(),
            shell: shell.to_string(),
            ..Default::default()
        }
        .with_declared(None);
        Self {
            os: info.platform,
            arch: arch.to_string(),
            shell: info.shell_kind,
        }
    }

    /// The machine this process runs on.
    pub fn local() -> Self {
        let shell = if cfg!(windows) { "pwsh" } else { "bash" };
        Self::reported(std::env::consts::OS, std::env::consts::ARCH, shell)
    }
}

/// Who provides a command found on a runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProvidedBy {
    /// The standard layer: under `$BUCKYOS_ROOT`, outside `tools/`.
    BuckyosRoot,
    /// An extension package: under `$BUCKYOS_ROOT/tools`.
    Exttools,
    /// The Agent's own (`tools/<skill>/bin`).
    Agent,
    /// Beside a development build of the program: serves as a standard helper,
    /// and views say so.
    Dev,
    /// The machine's own PATH.
    System,
}

/// Where a check found a command. `version` comes from a full check only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ResolvedCommand {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub provided_by: ProvidedBy,
    /// A full check found it does not run (exit 126 / 127): it counts as
    /// missing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stub: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtStatus {
    /// Every extended tool, all from extension packages.
    Ok,
    /// Some extended tools are missing.
    Partial,
    /// None of them is there.
    None,
    /// Every extended tool, some from the machine or the Agent.
    System,
}

/// The extended layer of a runtime, whatever it declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExtCheck {
    pub status: ExtStatus,
    #[serde(default)]
    pub missing: Vec<String>,
    /// Extended commands that are there but not from an extension package.
    #[serde(default)]
    pub system: Vec<String>,
}

impl ExtCheck {
    /// `ok | partial(missing a, b) | none | system`: a segment of a runtime's
    /// line in the plan table.
    pub fn summary(&self) -> String {
        match self.status {
            ExtStatus::Ok => "ok".to_string(),
            ExtStatus::Partial => format!("partial(missing {})", self.missing.join(", ")),
            ExtStatus::None => "none".to_string(),
            ExtStatus::System => "system".to_string(),
        }
    }
}

/// The last check of a runtime's tools ([`crate::baseline`]) against what it
/// declares: only a hint, the declaration decides (Q5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BaselineCheck {
    pub checked_at_ms: u64,
    /// The declaration checked against.
    #[serde(default = "standard_baseline")]
    pub level: RuntimeBaseline,
    /// Commands of that level not found on the runtime's PATH.
    #[serde(default)]
    pub missing: Vec<String>,
    /// Standard commands found outside `$BUCKYOS_ROOT`: not the standard ones.
    #[serde(default)]
    pub unmanaged: Vec<String>,
    /// Where each command found is, standard and extended.
    #[serde(default)]
    pub resolved: BTreeMap<String, ResolvedCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ext: Option<ExtCheck>,
    pub ok: bool,
    /// Why the check could not run (the runtime was not reachable).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl BaselineCheck {
    /// What is wrong, for a log line or a list; `None` when it passed.
    pub fn problem(&self) -> Option<String> {
        if self.ok {
            return None;
        }
        if let Some(error) = &self.error {
            return Some(error.clone());
        }
        let mut parts = Vec::new();
        if !self.missing.is_empty() {
            parts.push(format!("missing {}", self.missing.join(", ")));
        }
        if !self.unmanaged.is_empty() {
            parts.push(format!("not from $BUCKYOS_ROOT: {}", self.unmanaged.join(", ")));
        }
        Some(parts.join("; "))
    }

    /// The commands that serve from beside a development build.
    pub fn dev_helpers(&self) -> Vec<&str> {
        self.resolved
            .iter()
            .filter(|(_, c)| c.provided_by == ProvidedBy::Dev)
            .map(|(n, _)| n.as_str())
            .collect()
    }
}

/// Commands beyond the standard tools verified to exist on a runtime
/// (`check --commands`, skill installation).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ToolsCheck {
    pub checked_at_ms: u64,
    #[serde(default)]
    pub present: Vec<String>,
    #[serde(default)]
    pub missing: Vec<String>,
    /// Where each command found is.
    #[serde(default)]
    pub resolved: BTreeMap<String, ResolvedCommand>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// What the machine can do: learned when the runtime is checked, except
/// `baseline`, which its registrar declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeCaps {
    #[serde(flatten)]
    pub platform: RuntimePlatform,
    /// A desktop session the executor can observe or control.
    #[serde(default)]
    pub desktop: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_error: Option<String>,
    /// `current_user` / `switch_user`: how a node executes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_mode: Option<String>,
    /// Declared (Q5): decides the sessions' cmd_skills and what their
    /// prompts say.
    #[serde(default = "standard_baseline")]
    pub baseline: RuntimeBaseline,
    /// `$BUCKYOS_ROOT` of that machine as its executor reports it; a machine
    /// without a managed install has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub buckyos_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline_check: Option<BaselineCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_check: Option<ToolsCheck>,
}

fn standard_baseline() -> RuntimeBaseline {
    RuntimeBaseline::Standard
}

impl Default for RuntimeCaps {
    fn default() -> Self {
        Self {
            platform: RuntimePlatform::default(),
            desktop: false,
            desktop_error: None,
            identity_mode: None,
            baseline: RuntimeBaseline::Standard,
            buckyos_root: None,
            baseline_check: None,
            tools_check: None,
        }
    }
}

/// What this agent may do on the runtime: the node policy narrowed by the
/// owner, with the daemon URL's term. `roots` is the executor's boundary
/// (empty: unlimited); the agent's directories follow the directory
/// convention (`<owner home>/agents/<agent_id>` on the private runtime,
/// `<account home>/.opendan/<agent_id>` on a node) and are not recorded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeGrant {
    /// Executor operations granted; empty before the executor was asked.
    #[serde(default)]
    pub ops: Vec<ExecutorOp>,
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub computer: ComputerAccess,
    #[serde(default)]
    pub install: InstallGrant,
    /// Long-lived workspaces may be created there.
    #[serde(default)]
    pub workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

impl RuntimeGrant {
    /// Everything, on the agent's own machine.
    pub fn private() -> Self {
        Self {
            ops: ExecutorOp::ALL.to_vec(),
            roots: Vec::new(),
            computer: ComputerAccess::None,
            install: InstallGrant::System,
            workspace: true,
            expires_at_ms: None,
        }
    }

    /// No write operation is granted (`ops` known).
    pub fn read_only(&self) -> bool {
        !self.ops.is_empty() && !self.ops.iter().any(|op| op.is_mutating() && !op.is_computer())
    }

    pub fn can_exec(&self) -> bool {
        self.ops.is_empty() || self.ops.contains(&ExecutorOp::Exec)
    }

    pub fn expired_at(&self, now_ms: u64) -> bool {
        self.expires_at_ms.is_some_and(|t| t <= now_ms)
    }

    /// Whether `path` lies inside the granted roots (any path when there
    /// are none).
    pub fn within_roots(&self, path: &str) -> bool {
        if self.roots.is_empty() {
            return true;
        }
        let norm = |p: &str| p.replace('\\', "/").trim_end_matches('/').to_ascii_lowercase();
        let p = norm(path);
        self.roots.iter().any(|r| {
            let r = norm(&r.display().to_string());
            p == r || p.starts_with(&format!("{r}/"))
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAvailability {
    Available,
    RuntimeUnavailable,
    PermissionDenied,
}

/// `retired`: withdrawn, expired or unregistered; kept for its history,
/// never a candidate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeLifecycle {
    #[default]
    Active,
    Retired,
}

/// How the agent has used a runtime (the "runtimes used" view).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeUsage {
    #[serde(default)]
    pub sessions_total: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session_id: Option<String>,
    #[serde(default)]
    pub workspaces: u64,
    /// Skill commands installed there.
    #[serde(default)]
    pub skill_commands: Vec<String>,
}

/// One use of a runtime, noted in its `usage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuntimeUsageEvent {
    Session { session_id: String },
    Workspace,
    SkillCommand { name: String },
    SkillCommandRemoved { name: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeRecord {
    pub version: u32,
    pub runtime_id: String,
    pub name: String,
    /// One sentence for people and the model.
    #[serde(default)]
    pub desc: String,
    /// `native_host_id()` of the host that registered it.
    pub runtime_host: String,
    pub connector: RuntimeConnector,
    /// Switched on in the owner's settings (a disabled runtime keeps its
    /// record and says why it is unavailable).
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub caps: RuntimeCaps,
    #[serde(default)]
    pub grant: RuntimeGrant,
    pub availability: RuntimeAvailability,
    /// Created by the host; cannot be unregistered.
    #[serde(default)]
    pub builtin: bool,
    /// The owner's default runtime (`default_runtime_id`): the machine the
    /// owner means by "my computer" and where the owner's absolute paths
    /// are read; it may also be the private runtime.
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub lifecycle: RuntimeLifecycle,
    #[serde(default)]
    pub usage: RuntimeUsage,
    pub revision: u64,
    pub registered_by: String,
    pub registered_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub checked_at_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

fn default_true() -> bool {
    true
}

impl RuntimeRecord {
    pub fn names(&self, id: &str) -> bool {
        self.runtime_id == canonical_runtime(id) || matches!(&self.connector,
            RuntimeConnector::Native { aliases, .. } if aliases.iter().any(|alias| alias == id))
    }
    /// The `agent_tool` RuntimeConfig to execute through, when the runtime
    /// is switched on.
    pub fn exec(&self) -> Option<Value> {
        self.enabled.then(|| self.connector.exec())
    }

    /// A candidate for sessions and workspaces now.
    pub fn usable(&self) -> bool {
        self.lifecycle == RuntimeLifecycle::Active && self.enabled && self.availability == RuntimeAvailability::Available
    }

    pub fn is_private(&self) -> bool {
        self.connector.is_native()
    }
}

/// A runtime to register (`builtin` ones are registered by the host).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeRegister {
    pub runtime_id: String,
    pub connector: RuntimeConnector,
    pub name: String,
    #[serde(default)]
    pub desc: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub default: bool,
    #[serde(default = "standard_baseline")]
    pub baseline: RuntimeBaseline,
    #[serde(default)]
    pub grant: RuntimeGrant,
}

/// What an owner may change of a registration (CAS on `revision`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeUpdate {
    pub expected_revision: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub desc: Option<String>,
    #[serde(default)]
    pub roots: Option<Vec<PathBuf>>,
}

/// The private runtime as the host sees it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrivateRuntimeInfo {
    /// The Agent Service runs natively on its node: the private runtime is
    /// that process's environment, the same machine as `host`.
    pub native_service: bool,
    pub native_node_did: Option<String>,
    pub computer_endpoint: Option<String>,
    /// One sentence about where this process runs.
    pub note: String,
}

/// The runtimes of an agent as the system configures them: its private
/// runtime (always present and usable, declared `extended` in the aios
/// container, the owner's `host` declaration when the service runs
/// natively), the owner's desktop `host`, and every further runtime the
/// owner enabled in the agent's settings. A node runtime's roots and
/// platform are learned from the node (`describe`) when it is checked.
pub fn builtin_runtimes(settings: &AgentSettings, agent_did: &str, private: &PrivateRuntimeInfo) -> Vec<RuntimeRegister> {
    let default = canonical_runtime(settings.effective_default_runtime()).to_string();
    let mut aliases: Vec<String> = settings.runtimes.iter().filter_map(|(id, rs)| {
        match &rs.target {
            AgentRuntimeTarget::Node { node_did, .. }
                if private.native_service && private.native_node_did.as_ref() == Some(node_did) => Some(id.clone()),
            _ => None,
        }
    }).collect();
    aliases.sort_by_key(|id| (id != &default, id != HOST_RUNTIME, id.clone()));
    let owner = aliases.iter().find_map(|id| settings.runtimes.get(id));
    let computer = owner.and_then(|s| private.computer_endpoint.as_ref().map(|endpoint| {
        agent_tool::runtime::computer::NativeComputerConfig {
            endpoint: endpoint.clone(),
            node_did: private.native_node_did.clone().unwrap_or_default(),
            profile_id: owner_agent_profile_id(agent_did),
            access: if s.enabled { s.computer } else { ComputerAccess::None },
        }
    }));
    let mut grant = RuntimeGrant::private();
    grant.computer = computer.as_ref().map(|c| c.access).unwrap_or_default();
    let host_baseline = owner
        .and_then(|h| h.baseline)
        .unwrap_or(RuntimeBaseline::Standard);
    let mut out = vec![RuntimeRegister {
        runtime_id: PRIVATE_RUNTIME.into(),
        connector: RuntimeConnector::Native { aliases: aliases.clone(), computer },
        name: if aliases.is_empty() { "Private runtime" } else { "Owner's computer" }.into(),
        desc: if private.native_service {
            format!(
                "The Agent Service runs natively on this machine. Shell and files execute locally; desktop operations use the local driver over IPC. Private and owner runtime roles on this machine refer to this one runtime. {}",
                private.note
            )
        } else {
            format!("Your own environment: the agent's container. Commands run directly; paths here are not the owner's host paths. {}", private.note)
        }
        .trim()
        .to_string(),
        enabled: true,
        default: default == PRIVATE_RUNTIME || aliases.contains(&default),
        // The aios image: the standard tools plus its toolchains and media
        // tools (`build_aios` RUNTIME_PACKAGES, Node, Deno, uv).
        baseline: if private.native_service { host_baseline } else { RuntimeBaseline::Extended },
        grant,
    }];
    let mut has_host = false;
    for (id, rs) in &settings.runtimes {
        has_host |= id == HOST_RUNTIME;
        if aliases.contains(id) {
            continue;
        }
        let owner_grant = rs.grant.clone().unwrap_or_default();
        let (connector, name, desc) = match &rs.target {
            AgentRuntimeTarget::Node { node_id, node_did } => (
                RuntimeConnector::node(node_did, agent_did),
                if id == HOST_RUNTIME { "Owner's desktop".to_string() } else { id.clone() },
                if id == HOST_RUNTIME {
                    format!("The owner's own machine (node `{node_id}`), shared with the owner: the owner's files and tools are here; commands and files go through its node executor.")
                } else {
                    format!("Node `{node_id}` ({node_did}); commands and files go through its node executor.")
                },
            ),
            AgentRuntimeTarget::Executor { executor_id, profile_id } => (
                RuntimeConnector::executor(executor_id, profile_id.as_deref()),
                id.clone(),
                format!("Standalone executor `{executor_id}` (a device joined by daemon URL); commands and files go through its executor."),
            ),
        };
        let desc = if rs.enabled {
            desc
        } else {
            "Switched off in the agent's settings; enable it there to use it.".to_string()
        };
        out.push(RuntimeRegister {
            runtime_id: id.clone(),
            connector,
            name,
            desc,
            enabled: rs.enabled,
            default: default == *id,
            baseline: rs.baseline.unwrap_or(RuntimeBaseline::Standard),
            grant: RuntimeGrant {
                ops: Vec::new(),
                roots: owner_grant.roots.iter().map(PathBuf::from).collect(),
                computer: rs.computer,
                install: owner_grant.install,
                workspace: owner_grant.workspace.unwrap_or(id == HOST_RUNTIME),
                expires_at_ms: owner_grant.expires_at_ms,
            },
        });
    }
    if !has_host {
        out.push(RuntimeRegister {
            runtime_id: HOST_RUNTIME.into(),
            connector: RuntimeConnector::NodeExecutor { node_did: None, executor_id: None, profile_id: String::new() },
            name: "Owner's desktop".into(),
            desc: "The owner's desktop. Not enabled for this agent: the node has no `owner_agents` policy, or the agent's settings do not list `host`.".into(),
            enabled: false,
            default: false,
            baseline: RuntimeBaseline::Standard,
            grant: RuntimeGrant::default(),
        });
    }
    out
}

/// The directories the private runtime's `$BUCKYOS_ROOT` must have (the
/// directory convention): missing ones are a deployment error, never
/// "unavailable".
pub fn private_layout_missing(buckyos_root: &Path, owner: &str, agent_id: &str) -> Vec<PathBuf> {
    [
        buckyos_root.join("bin"),
        buckyos_root.join("data").join("home").join(owner).join("agents").join(agent_id),
        buckyos_root.join("instance"),
        buckyos_root.join("tools"),
    ]
    .into_iter()
    .filter(|p| !p.is_dir())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_resolve_to_the_private_runtime() {
        assert_eq!(canonical_runtime("container"), PRIVATE_RUNTIME);
        assert_eq!(canonical_runtime("local"), PRIVATE_RUNTIME);
        assert_eq!(canonical_runtime("host"), "host");
        assert!(is_private_runtime("local") && !is_private_runtime("host"));
    }

    #[test]
    fn grants_tell_read_only_and_roots() {
        let mut g = RuntimeGrant::private();
        assert!(!g.read_only() && g.can_exec() && g.within_roots("/anywhere"));
        g.ops = vec![ExecutorOp::Read, ExecutorOp::Stat, ExecutorOp::ComputerObserve];
        assert!(g.read_only() && !g.can_exec());
        g.roots = vec![PathBuf::from("/home/o"), PathBuf::from(r"C:\Users\o")];
        assert!(g.within_roots("/home/o/x") && g.within_roots(r"c:\users\o\d") && !g.within_roots("/home/other"));
        assert!(RuntimeGrant::default().can_exec() && !RuntimeGrant::default().read_only(), "unknown ops refuse nothing");
    }

    #[test]
    fn connectors_execute_through_their_runtime_config() {
        let node = RuntimeConnector::node("did:dev:n", "did:web:a");
        assert_eq!(node.exec()["kind"], "remote_node");
        assert!(node.exec()["remote_node"]["profile_id"].as_str().unwrap().starts_with("agent-"));
        let exec = RuntimeConnector::executor("dev-1", None);
        assert_eq!(exec.executor_id(), Some("dev-1"));
        assert_eq!(RuntimeConnector::Native { aliases: Vec::new(), computer: None }.exec(), serde_json::json!({ "kind": "native" }));
    }
}
