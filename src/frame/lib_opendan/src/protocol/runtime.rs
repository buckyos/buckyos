//! Runtime descriptor (§7.1). Declared by the runtime provider; not stored in
//! the session directory except through `binding.json`.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FsView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// Extra roots the runtime can see (e.g. an app data directory).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Capabilities {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tools: BTreeMap<String, String>,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub os: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeDescriptor {
    pub runtime_id: String,
    /// `native | tmux`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// `opendan | app:<app_id>`.
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub fs_view: FsView,
    /// PATH layers in front of the inherited PATH (`<sid>/.runtime/bin` first).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_layers: Vec<String>,
    #[serde(default)]
    pub capabilities: Capabilities,
}
