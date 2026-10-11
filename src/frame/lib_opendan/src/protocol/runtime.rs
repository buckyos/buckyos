//! Runtimes of an agent (`state/runtimes/<runtime_id>.json`): where its
//! sessions and workspaces can run. A registration names a runtime and says
//! whether this host can use it; executing in it is the runtime layer's job
//! (`agent_tool::runtime`).

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use agent_tool::runtime::{Capabilities, RuntimeDescriptor};

pub const RUNTIME_RECORD_VERSION: u32 = 1;

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
    HostNative,
    RemoteSsh,
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

/// The runtimes every agent has: its own environment and the machine
/// under it. `roots` are suggestions for workspaces in the container.
pub fn builtin_runtimes(container_roots: Vec<PathBuf>, container_note: &str) -> Vec<RuntimeRegister> {
    vec![
        RuntimeRegister {
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
        },
        RuntimeRegister {
            runtime_id: HOST_RUNTIME.into(),
            kind: RuntimeKind::HostNative,
            name: "Host machine".into(),
            description: "The machine the agent's container runs on, where the owner's own files and tools are. Not usable yet: it needs a host executor.".into(),
            exec: None,
            roots: Vec::new(),
            capabilities: serde_json::json!({}),
        },
    ]
}
