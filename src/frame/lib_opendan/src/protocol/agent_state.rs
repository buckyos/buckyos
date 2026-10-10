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
    /// What a waiting session waits for (a parent tells "needs input" from
    /// "waits for a tool / its own sub sessions").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_for: Option<super::state::WaitingKind>,
    /// A Turn is open (work in progress, or waiting inside it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub turn_open: bool,
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
    /// [`super::state::SessionState::takes_input`] as last reported.
    pub fn takes_input(&self) -> bool {
        self.run_state != RunState::Finished
            || matches!(self.acceptance, Acceptance::Pending | Acceptance::Accepted)
            || self.pending_decision.is_some()
    }

    pub fn created(now_ms: u64) -> Self {
        Self {
            rev: 0,
            run_state: RunState::Created,
            outcome: None,
            acceptance: Acceptance::NotApplicable,
            one_line_status: String::new(),
            report_brief: String::new(),
            pending_decision: None,
            waiting_for: None,
            turn_open: false,
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
    /// What the session is bound to (a ui session: its mailbox address).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
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

/// One line of `state/perception/<sid>.jsonl` (Memory requirements A.5).
///
/// Runtime records (`run_digest`, `task_outcome`, `task_discarded`) only feed
/// consolidation; `observation` records are what Sessions write. After a
/// consolidation disposes a record, cleanup keeps the line with its identity,
/// key, source and `cleared` marker and drops the body.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PerceptionRecord {
    pub seq: u64,
    pub at_ms: u64,
    pub session_id: String,
    /// `run_digest | observation | task_outcome | task_discarded`.
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
    /// Writer-given replay key, unique within the session file (A.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Digest of the observed content, kept after cleanup so a replay with
    /// different content is still a conflict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    /// Subjects (bound by the host) and object range of the observation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<agent_tool::agent_memory::Scope>,
    /// Classification hint (`preference`, `correction`, …); not a cognition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_kind: Option<String>,
    /// `explicit` when the user asked to remember.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_intent: Option<String>,
    /// Cognitions (`item_…@rev`) this observation was derived from or
    /// corrects (echo and correction references, E-15, §4.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cites: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchors: Option<PerceptionAnchors>,
    /// When the described thing happened (may be an estimate); `at_ms` is
    /// when it was observed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    /// Original event (session event, tool call, task / goal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<agent_tool::agent_memory::SourceRef>,
    /// Raw mentions and the candidates the component found for them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<Mention>,
    /// Synthesized later (a missing run digest), not an original observation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub backfilled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cleared: Option<ClearedMarker>,
}

/// Interpretation anchors of an observation (§4.2); the intent's own anchor
/// wins over the environment's.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PerceptionAnchors {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent_timezone: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Mention {
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<MentionCandidate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MentionCandidate {
    pub object_id: String,
    pub canonical_name: String,
    /// Why it is a candidate (`alias:<text>`).
    pub basis: String,
}

/// What remains of a disposed record after cleanup.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ClearedMarker {
    /// `absorbed | duplicate | discarded`.
    pub outcome: String,
    pub occasion_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cognition_refs: Vec<String>,
    pub at_ms: u64,
}

impl PerceptionRecord {
    /// `<sid>:<seq>`.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.session_id, self.seq)
    }

    /// Written by the Runtime, not observed by a Session (A.9).
    pub fn is_runtime(&self) -> bool {
        matches!(self.kind.as_str(), "run_digest" | "task_outcome" | "task_discarded")
    }
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
