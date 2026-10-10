use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const WORKSPACE_METADATA_FILE: &str = ".opendan-workspace.json";
pub const WORKSPACE_METADATA_VERSION: u32 = 1;
pub const WORKSPACE_RECORD_VERSION: u32 = 1;
pub const LOCAL_WORKSPACE_RUNTIME: &str = "local";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceLocation {
    pub runtime_id: String,
    pub directory: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceUsage {
    Private,
    #[default]
    Collaborative,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceLifecycle {
    #[default]
    Active,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAvailability {
    Available,
    Missing,
    RuntimeUnavailable,
    PermissionDenied,
    InvalidMetadata,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceMetadata {
    pub version: u32,
    pub workspace_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceConflict {
    pub code: String,
    pub message: String,
    pub candidate: WorkspaceLocation,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceRecord {
    pub version: u32,
    pub runtime_host: String,
    pub workspace_id: String,
    pub name: String,
    pub description: String,
    pub location: WorkspaceLocation,
    pub usage: WorkspaceUsage,
    pub lifecycle: WorkspaceLifecycle,
    pub availability: WorkspaceAvailability,
    pub revision: u64,
    pub location_revision: u64,
    pub created_at_ms: u64,
    pub registered_at_ms: u64,
    pub updated_at_ms: u64,
    pub checked_at_ms: Option<u64>,
    pub registered_by: String,
    pub source: String,
    #[serde(default)]
    pub private_notes: String,
    #[serde(default)]
    pub source_session: Option<String>,
    #[serde(default)]
    pub policy_ref: Option<String>,
    #[serde(default)]
    pub conflict: Option<WorkspaceConflict>,
    #[serde(default)]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceCreate {
    pub operation_id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub location: WorkspaceLocation,
    #[serde(default)]
    pub usage: WorkspaceUsage,
    #[serde(default)]
    pub source_session: Option<String>,
    #[serde(default)]
    pub policy_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceImport {
    pub operation_id: String,
    pub location: WorkspaceLocation,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub usage: WorkspaceUsage,
    #[serde(default)]
    pub expected_revision: Option<u64>,
    #[serde(default)]
    pub source_session: Option<String>,
    #[serde(default)]
    pub policy_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceDiscover {
    #[serde(default)]
    pub expected_workspace_id: Option<String>,
    pub location: WorkspaceLocation,
    #[serde(default)]
    pub expected_revision: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceQuery {
    #[serde(default)]
    pub runtime_id: Option<String>,
    #[serde(default)]
    pub lifecycle: Option<WorkspaceLifecycle>,
    #[serde(default)]
    pub availability: Option<WorkspaceAvailability>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceUpdate {
    pub expected_revision: u64,
    #[serde(default)]
    pub lifecycle: Option<WorkspaceLifecycle>,
    #[serde(default)]
    pub private_notes: Option<String>,
}
