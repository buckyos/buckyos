//! Runtimes of an agent (`state/runtimes/<runtime_id>.json`): where its
//! sessions and workspaces can run. A registration names a runtime and says
//! whether this host can use it; executing in it is the runtime layer's job
//! (`agent_tool::runtime`).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use agent_tool::runtime::{Capabilities, RuntimeDescriptor};
use buckyos_api::{owner_agent_profile_id, AgentRuntimeTarget, AgentSettings, ComputerAccess};

pub const RUNTIME_RECORD_VERSION: u32 = 2;

/// The environment the agent process itself runs in (its container; the
/// host process under `debug_jarvis.sh` / `--dev`).
pub const CONTAINER_RUNTIME: &str = "container";
/// The machine the agent's container runs on.
pub const HOST_RUNTIME: &str = "host";
/// Former name of [`CONTAINER_RUNTIME`] in workspace locations, still read.
pub const LEGACY_LOCAL_RUNTIME: &str = "local";

/// The runtime a location's id names, aliases resolved.
pub fn canonical_runtime(runtime_id: &str) -> &str {
    if runtime_id == LEGACY_LOCAL_RUNTIME {
        CONTAINER_RUNTIME
    } else {
        runtime_id
    }
}

/// Runs in this process's own environment (no executor in between).
pub fn is_native_runtime(runtime_id: &str) -> bool {
    canonical_runtime(runtime_id) == CONTAINER_RUNTIME
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Container,
    /// The owner's desktop host (`host`): a node's embedded executor.
    HostNative,
    /// Another node's embedded executor.
    RemoteNode,
    RemoteSsh,
}

impl RuntimeKind {
    /// Executed through a Node Executor (`remote_node` runtime config).
    pub fn is_node(self) -> bool {
        matches!(self, RuntimeKind::HostNative | RuntimeKind::RemoteNode)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeAvailability {
    Available,
    RuntimeUnavailable,
    PermissionDenied,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeRecord {
    pub version: u32,
    pub runtime_id: String,
    pub kind: RuntimeKind,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// `native_host_id()` of the host that registered it.
    pub runtime_host: String,
    /// How the runtime layer reaches it (`agent_tool` RuntimeConfig);
    /// `None`: there is no executor for it yet.
    #[serde(default)]
    pub exec: Option<Value>,
    /// Where workspaces are best placed in it.
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    /// `{tools, network, os}` as far as known.
    #[serde(default)]
    pub capabilities: Value,
    pub availability: RuntimeAvailability,
    /// Created by the host; cannot be unregistered.
    #[serde(default)]
    pub builtin: bool,
    /// The runtime new sessions without a workspace run in (the owner's
    /// `default_runtime_id`); exactly one record carries it.
    #[serde(default)]
    pub default: bool,
    /// ComputerUse the owner granted the agent on this runtime.
    #[serde(default)]
    pub computer: ComputerAccess,
    pub revision: u64,
    pub registered_by: String,
    pub registered_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub checked_at_ms: Option<u64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

/// A runtime to register (`builtin` ones are registered by the host).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeRegister {
    pub runtime_id: String,
    pub kind: RuntimeKind,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub exec: Option<Value>,
    #[serde(default)]
    pub roots: Vec<PathBuf>,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub computer: ComputerAccess,
}

/// What an owner may change of a registration (CAS on `revision`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeUpdate {
    pub expected_revision: u64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub roots: Option<Vec<PathBuf>>,
}

/// The `remote_node` runtime configuration an owner-enabled node runtime
/// executes through (`agent_tool::runtime::RuntimeConfig`): the node and the
/// profile the scheduler derives for the agent there.
pub fn node_exec(node_did: &str, agent_did: &str) -> Value {
    serde_json::json!({
        "kind": agent_tool::runtime::node::REMOTE_NODE_KIND,
        "remote_node": { "node_did": node_did, "profile_id": owner_agent_profile_id(agent_did) },
    })
}

/// The runtimes of an agent as the system configures them (integration
/// TODO §3.1): its own environment, the owner's desktop `host`, and every
/// further runtime the owner enabled in the agent's settings. `roots` are
/// suggestions for workspaces in the container; a node runtime's roots are
/// learned from the node (`describe`) when it is checked.
pub fn builtin_runtimes(
    container_roots: Vec<PathBuf>,
    container_note: &str,
    settings: &AgentSettings,
    agent_did: &str,
) -> Vec<RuntimeRegister> {
    let default = settings.effective_default_runtime();
    let mut out = vec![RuntimeRegister {
        runtime_id: CONTAINER_RUNTIME.into(),
        kind: RuntimeKind::Container,
        name: "Agent container".into(),
        description: format!(
            "The environment the agent itself runs in; commands run directly. {container_note}"
        )
        .trim()
        .to_string(),
        exec: Some(serde_json::json!({ "kind": "native" })),
        roots: container_roots,
        capabilities: serde_json::json!({ "network": true, "os": std::env::consts::OS }),
        default: default == CONTAINER_RUNTIME,
        computer: ComputerAccess::None,
    }];
    let mut has_host = false;
    for (id, rs) in &settings.runtimes {
        has_host |= id == HOST_RUNTIME;
        let (kind, name) = if id == HOST_RUNTIME {
            (RuntimeKind::HostNative, "Host machine".to_string())
        } else {
            (RuntimeKind::RemoteNode, id.clone())
        };
        let (exec, description, capabilities) = match (&rs.target, rs.enabled) {
            (_, false) => (
                None,
                "Switched off in the agent's settings; enable it there to use it.".to_string(),
                serde_json::json!({ "enabled": false }),
            ),
            (AgentRuntimeTarget::Node { node_id, node_did }, true) => (
                Some(node_exec(node_did, agent_did)),
                format!(
                    "Node `{node_id}` ({node_did}): the owner's machine, where the owner's own files and tools are; commands and files go through its node executor."
                ),
                serde_json::json!({ "enabled": true, "node_id": node_id, "node_did": node_did, "computer": rs.computer }),
            ),
            (AgentRuntimeTarget::Executor { executor_id }, true) => (
                None,
                format!("Standalone executor `{executor_id}`: not wired into Sessions yet (use `executor_call`)."),
                serde_json::json!({ "enabled": true, "executor_id": executor_id, "computer": rs.computer }),
            ),
        };
        out.push(RuntimeRegister {
            runtime_id: id.clone(),
            kind,
            name,
            description,
            exec,
            roots: Vec::new(),
            capabilities,
            default: default == id,
            computer: rs.computer,
        });
    }
    if !has_host {
        out.push(RuntimeRegister {
            runtime_id: HOST_RUNTIME.into(),
            kind: RuntimeKind::HostNative,
            name: "Host machine".into(),
            description: "The owner's desktop. Not enabled for this agent: the node has no `owner_agents` policy, or the agent's settings do not list `host`.".into(),
            exec: None,
            roots: Vec::new(),
            capabilities: serde_json::json!({ "enabled": false }),
            default: false,
            computer: ComputerAccess::None,
        });
    }
    out
}
