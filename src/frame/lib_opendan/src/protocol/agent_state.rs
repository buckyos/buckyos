//! Agent State files on the AgentRoot (§6): session registry, perception,
//! artifact list.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::config::{AgentAccess, CreatedBy, DriverRef, Origin, SessionKind, WorkspaceRef};
use super::state::{Acceptance, Activity, Outcome, RunState};

/// Driver of the last commit (Q9: which runner advanced the session).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastRunner {
    pub runner_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub pid: u32,
    pub lock_epoch: u64,
    pub at_ms: u64,
}

/// Cached status of a session (the truth is its own state.json).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SessionStatus {
    /// = state.json `rev`; updates with `rev ≤` the stored one are ignored.
    pub rev: u64,
    pub run_state: RunState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub acceptance: Acceptance,
    #[serde(default)]
    pub one_line_status: String,
    /// ≤ 500 chars.
    #[serde(default)]
    pub report_brief: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_decision: Option<Value>,
    #[serde(default)]
    pub activity: Activity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_runner: Option<LastRunner>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<Value>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl SessionStatus {
    pub fn created(now_ms: u64) -> Self {
        Self {
            rev: 0,
            run_state: RunState::Created,
            outcome: None,
            acceptance: Acceptance::NotApplicable,
            one_line_status: String::new(),
            report_brief: String::new(),
            pending_decision: None,
            activity: Activity::default(),
            last_runner: None,
            last_error: None,
            updated_at_ms: now_ms,
        }
    }
}

/// `state/sessions/<sid>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct RegistryEntry {
    pub session_id: String,
    pub kind: SessionKind,
    #[serde(default)]
    pub class: String,
    pub created_by: CreatedBy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub driver: DriverRef,
    /// Path of the session directory (DFS path).
    pub location: String,
    /// kmsg queue urn other parties post to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_queue: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wake_event: Option<String>,
    #[serde(default)]
    pub agent_access: AgentAccess,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub objective: String,
    /// Initial activity declaration copied from the session scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<super::config::Scope>,
    pub status: SessionStatus,
    /// `verify` marks entries whose location disappeared (never deleted).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unreachable: bool,
    /// Location changes (session moved); last write wins, driver only.
    #[serde(default)]
    pub location_rev: u64,
}

/// One line of `state/perception/<sid>.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PerceptionRecord {
    pub seq: u64,
    pub at_ms: u64,
    pub session_id: String,
    /// `round_digest | observation | task_outcome | task_discarded`.
    pub kind: String,
    /// `session | self`.
    #[serde(default = "session_source")]
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub refs: Value,
}

fn session_source() -> String {
    "session".to_string()
}

/// `state/perception/.cursor.json` — consolidation offsets per session file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PerceptionCursor {
    #[serde(default)]
    pub offsets: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// `state/artifacts/<aid>/artifact.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactHead {
    pub aid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceRef>,
    /// Version the user accepted last (`null` = none valid).
    #[serde(default)]
    pub head: Option<String>,
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum VersionState {
    Produced,
    Accepted,
    Discarded,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SideEffectRef {
    pub call_id: String,
    pub tool: String,
    #[serde(default)]
    pub note: String,
}

/// `state/artifacts/<aid>/versions/v-<sid>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactVersion {
    pub ver: String,
    pub session: String,
    #[serde(default)]
    pub base: Option<String>,
    pub state: VersionState,
    /// One-off outputs inside the session directory (relative paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// Workspace change reference given by the workspace mechanism.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub workspace_ref: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub side_effects: Vec<SideEffectRef>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

/// Result of a discard decision (S-26).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DiscardReport {
    /// What the workspace mechanism reverted (empty when unsupported).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reverted: Vec<String>,
    /// `supported | unsupported | none`.
    #[serde(default)]
    pub workspace: String,
    /// Irreversible items, listed one by one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<SideEffectRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_moved_to: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RegistryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<SessionKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_state_in: Vec<RunState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_finished_or_pending: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_decision: Option<bool>,
}

impl Default for RegistryQuery {
    fn default() -> Self {
        Self {
            kind: None,
            driver: None,
            run_state_in: Vec::new(),
            not_finished_or_pending: None,
            artifact_id: None,
            pending_decision: None,
        }
    }
}
