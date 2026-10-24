//! Goals on the AgentRoot (`state/goals/<goal_id>/`): the Goal object, the
//! Goal State its Goal Session keeps, the Activation records and the
//! schedule progress of the host.
//!
//! ```text
//! state/goals/<goal_id>/
//!   goal.json              definition + status + current epoch   (GoalStore, CAS)
//!   revisions/<rev>.json   immutable snapshot per content revision (GoalStore)
//!   changes.jsonl          every change, with who / via / at / reason (GoalStore)
//!   state.json             Goal State                             (driver of the epoch's Goal Session)
//!   activations.jsonl      Activation records                     (same driver, idempotent by (epoch, seq))
//!   epochs/<n>/state.json  Goal State archived when epoch n ended (host)
//!   schedule.json          schedule progress                      (host)
//! ```
//!
//! A Goal is the truth; its Goal Session is derived from `(goal_id, epoch)`
//! and holds no copy of the definition. One Activation is one logical Turn
//! of the Goal Session.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::state::Outcome;

pub const GOAL_SCHEMA: &str = "opendan.goal/1";
pub const GOAL_STATE_SCHEMA: &str = "opendan.goal_state/1";
pub const GOAL_ACTIVATION_SCHEMA: &str = "opendan.goal_activation/1";
pub const GOAL_SCHEDULE_SCHEMA: &str = "opendan.goal_schedule/1";

/// Source id of the events the runtime posts to a Goal Session
/// (`source = system:goal`).
pub const GOAL_SOURCE_ID: &str = "goal";
/// The implicit active subscription of a Goal Session on them.
pub const GOAL_ACTIVATION_SUBSCRIPTION: &str = "_goal_activation";
/// Part name of the Goal header message in an input receipt.
pub const PART_GOAL: &str = "goal_activation";
/// `receipt.extra` key carrying the Activation a batch opened.
pub const RECEIPT_GOAL_ACTIVATION: &str = "goal_activation";
/// `receipt.extra` key carrying the reasons a batch adds to the open
/// Activation it joins.
pub const RECEIPT_GOAL_REASONS: &str = "goal_reasons";

/// Recent entries kept in the bounded lists of a Goal State.
pub const GOAL_ABSORBED_KEEP: usize = 512;
pub const GOAL_RECENT_WORKS_KEEP: usize = 16;
pub const GOAL_QUESTIONS_KEEP: usize = 16;
pub const GOAL_SKIPPED_KEEP: usize = 16;
/// Completed consolidation receipts kept in `GoalState.maintenance`.
pub const MAINTENANCE_RECEIPTS_KEEP: usize = 32;
/// Batches (receipts) one Activation may open.
pub const MAINTENANCE_BATCHES_PER_ACTIVATION: usize = 3;

/// `GoalDefinition.extensions` key of the host's trusted marks.
pub const OPENDAN_EXTENSION: &str = "opendan";
/// `extensions.opendan.builtin` of the agent's default consolidation Goal.
pub const BUILTIN_MEMORY_MAINTENANCE: &str = "memory_maintenance";
/// `GoalCreate.key` the host seeds it with (unique per Agent directory).
pub const BUILTIN_MEMORY_MAINTENANCE_KEY: &str = "builtin:memory_maintenance";
/// Capability that opens the `memory_consolidation_begin/commit` host tools
/// and binds the drive to the consolidation lease.
pub const CAP_MEMORY_CONSOLIDATION: &str = "memory_consolidation";
/// Policy version of the seeded definition (a higher one revises the marks
/// of an existing Goal; never the owner's budget, schedule or status).
pub const MEMORY_MAINTENANCE_POLICY_VERSION: u64 = 1;

/// `extensions.opendan` of a Goal: trusted configuration written by the
/// host's seeding path only (a UI, Goal or Work caller may not set it).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpendanGoalExtension {
    /// Stable identity of a built-in Goal (`memory_maintenance`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default)]
    pub policy_version: u64,
    /// Host capabilities of its Goal Session (`memory_consolidation`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Whether its Goal Session may record perceptions (the consolidation
    /// Goal must not feed itself).
    #[serde(default = "default_true")]
    pub records_perception: bool,
}

fn default_true() -> bool {
    true
}

impl OpendanGoalExtension {
    pub fn memory_maintenance() -> Self {
        Self {
            builtin: Some(BUILTIN_MEMORY_MAINTENANCE.to_string()),
            policy_version: MEMORY_MAINTENANCE_POLICY_VERSION,
            capabilities: vec![CAP_MEMORY_CONSOLIDATION.to_string()],
            records_perception: false,
        }
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.capabilities.iter().any(|c| c == cap)
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.builtin.as_deref() == Some(name)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalLifecycle {
    /// Has a reachable end (`achieved_when`).
    Achieve,
    /// Keeps a state (`healthy_state`); never achieved.
    #[default]
    Maintain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    Active,
    Paused,
    Achieved,
    Archived,
}

impl GoalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            GoalStatus::Active => "active",
            GoalStatus::Paused => "paused",
            GoalStatus::Achieved => "achieved",
            GoalStatus::Archived => "archived",
        }
    }

    /// The Goal Session of the current epoch is alive.
    pub fn is_live(&self) -> bool {
        matches!(self, GoalStatus::Active | GoalStatus::Paused)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalScope {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
    /// Workspace the Goal Session (and the Work it creates) works in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// A method / capability the Goal works with (architecture §10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct GoalSkill {
    pub name: String,
    /// Where it is read (path, object id, behavior name).
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl<'de> Deserialize<'de> for GoalSkill {
    /// A bare string is a skill by name.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            name: String,
            #[serde(default, rename = "ref")]
            reference: Option<String>,
            #[serde(default)]
            version: Option<String>,
            #[serde(default)]
            note: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Name(String),
            Full(Full),
        }
        Ok(match Either::deserialize(d)? {
            Either::Name(name) => GoalSkill {
                name,
                reference: None,
                version: None,
                note: None,
            },
            Either::Full(f) => GoalSkill {
                name: f.name,
                reference: f.reference,
                version: f.version,
                note: f.note,
            },
        })
    }
}

/// Hard limits of one Activation (accumulated over its runs).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActivationBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
}

/// Limits of one statistics period (fixed windows of `window_secs`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PeriodBudget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_secs: Option<u64>,
    /// Tokens of Activations and absorbed Work together.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_activations: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_works: Option<u32>,
}

/// Budget of a Goal; a field left out takes `agent.toml [session.goal]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalBudget {
    #[serde(default)]
    pub activation: ActivationBudget,
    #[serde(default)]
    pub period: PeriodBudget,
}

impl GoalBudget {
    pub fn is_empty(&self) -> bool {
        self == &GoalBudget::default()
    }

    /// `self` over `base`, field by field.
    pub fn over(&self, base: &GoalBudget) -> GoalBudget {
        GoalBudget {
            activation: ActivationBudget {
                max_total_tokens: self
                    .activation
                    .max_total_tokens
                    .or(base.activation.max_total_tokens),
                max_runs: self.activation.max_runs.or(base.activation.max_runs),
            },
            period: PeriodBudget {
                window_secs: self.period.window_secs.or(base.period.window_secs),
                max_total_tokens: self
                    .period
                    .max_total_tokens
                    .or(base.period.max_total_tokens),
                max_activations: self.period.max_activations.or(base.period.max_activations),
                max_works: self.period.max_works.or(base.period.max_works),
            },
        }
    }
}

/// When the runtime offers the Goal a run (phase 1: a fixed interval).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalScheduleSpec {
    /// Scheduled Activations every `every_secs` (none: manual and event
    /// driven only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    /// A scheduled fire closer than this to the last Activation is skipped
    /// (`None`: `[session.goal] min_interval_secs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_interval_secs: Option<u64>,
    /// The first Activation starts when the Goal Session is created.
    #[serde(default)]
    pub start_on_create: bool,
}

impl GoalScheduleSpec {
    pub fn is_empty(&self) -> bool {
        self == &GoalScheduleSpec::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalCreatedBy {
    pub principal: String,
    /// `owner | ui_session:<sid> | opendan`.
    pub via: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalSessionRef {
    pub epoch: u64,
    pub sid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpochStart {
    Created,
    Reopened,
    Restarted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EpochEnd {
    Archived,
    Achieved,
    /// A restart opened the next epoch (the Work keeps running).
    Restarted,
}

/// What a new epoch keeps of the previous Goal State.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StateKeep {
    /// Everything (reopen's default).
    #[default]
    All,
    /// Only what must not be lost: `active_works`, `absorbed`, `usage`,
    /// `recent_works`; skills return to `initial_skills` (restart's
    /// default).
    Facts,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalEpoch {
    pub epoch: u64,
    pub sid: String,
    pub started_at_ms: u64,
    pub start: EpochStart,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default)]
    pub keep: StateKeep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<EpochEnd>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
}

/// `goal.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalDefinition {
    pub schema: String,
    pub goal_id: String,
    /// Content revision: definition, budget, schedule. Status changes,
    /// reopen and restart do not count.
    pub revision: u64,
    /// Every change of this file (`changes.jsonl` seq of the last one).
    #[serde(default)]
    pub change_seq: u64,
    pub title: String,
    pub objective: String,
    #[serde(default)]
    pub scope: GoalScope,
    #[serde(default)]
    pub lifecycle: GoalLifecycle,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub achieved_when: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthy_state: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub distance_evaluation: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub initial_skills: Vec<GoalSkill>,
    pub owner: String,
    pub created_by: GoalCreatedBy,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub beneficiaries: Vec<String>,
    #[serde(default, skip_serializing_if = "GoalBudget::is_empty")]
    pub budget: GoalBudget,
    #[serde(default, skip_serializing_if = "GoalScheduleSpec::is_empty")]
    pub schedule: GoalScheduleSpec,
    /// Behavior of the Goal Session (`None`: `[session.goal]
    /// default_behavior`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub behavior: Option<String>,
    pub status: GoalStatus,
    #[serde(default)]
    pub status_changed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_by: Option<String>,
    /// The epoch whose Goal Session is current.
    pub session: GoalSessionRef,
    #[serde(default)]
    pub epochs: Vec<GoalEpoch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    /// Fields of later phases, preserved verbatim.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

impl GoalDefinition {
    /// The host's marks (`None`: an ordinary Goal).
    pub fn opendan(&self) -> Option<OpendanGoalExtension> {
        self.extensions
            .get(OPENDAN_EXTENSION)
            .and_then(|v| serde_json::from_value(v.clone()).ok())
    }

    pub fn is_builtin(&self, name: &str) -> bool {
        self.opendan().is_some_and(|e| e.is_builtin(name))
    }

    pub fn has_capability(&self, cap: &str) -> bool {
        self.opendan().is_some_and(|e| e.has_capability(cap))
    }

    pub fn epoch(&self, n: u64) -> Option<&GoalEpoch> {
        self.epochs.iter().find(|e| e.epoch == n)
    }

    pub fn current_epoch(&self) -> Option<&GoalEpoch> {
        self.epoch(self.session.epoch)
    }
}

/// One line of `changes.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalChange {
    pub seq: u64,
    pub at_ms: u64,
    /// `created | revised | status | reopened | restarted | epoch_ended`.
    pub kind: String,
    pub who: String,
    /// `owner | ui_session:<sid> | goal_session:<sid> | opendan`.
    pub via: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub detail: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivationReasonKind {
    Created,
    Scheduled,
    /// A Work of the Goal finished (or needs attention).
    Child,
    /// A message handed over from a UI.
    Feedback,
    Manual,
    Reopened,
    Restarted,
    /// Any other subscribed event (a followed task, an object).
    Event,
    /// A question of the Goal was answered (or closed) by the user:
    /// `ref` is its notification.
    Answer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActivationReason {
    pub kind: ActivationReasonKind,
    /// Input key.
    pub key: String,
    /// Work sid (`child`), message key (`feedback`).
    #[serde(default, rename = "ref", skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Slots that passed while no fire was possible (`scheduled`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub missed: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivationOutcome {
    Completed,
    Failed,
    BudgetExhausted,
    Stopped,
    NeedsInput,
}

impl ActivationOutcome {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActivationOutcome::Completed => "completed",
            ActivationOutcome::Failed => "failed",
            ActivationOutcome::BudgetExhausted => "budget_exhausted",
            ActivationOutcome::Stopped => "stopped",
            ActivationOutcome::NeedsInput => "needs_input",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivationDecision {
    /// Looked, nothing to do now (not "achieved").
    NoAction,
    Acted,
    CreatedWork,
    NeedsInfo,
    /// No valid result was submitted.
    Unknown,
}

impl ActivationDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            ActivationDecision::NoAction => "no_action",
            ActivationDecision::Acted => "acted",
            ActivationDecision::CreatedWork => "created_work",
            ActivationDecision::NeedsInfo => "needs_info",
            ActivationDecision::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActivationCost {
    #[serde(default)]
    pub runs: u32,
    #[serde(default)]
    pub total_tokens: u64,
}

/// One line of `activations.jsonl`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalActivation {
    pub schema: String,
    pub goal_id: String,
    pub epoch: u64,
    /// = the Turn index of the epoch's Goal Session.
    pub seq: u64,
    pub session_id: String,
    pub goal_revision: u64,
    pub started_at_ms: u64,
    pub ended_at_ms: u64,
    #[serde(default)]
    pub reasons: Vec<ActivationReason>,
    pub outcome: ActivationOutcome,
    pub decision: ActivationDecision,
    #[serde(default)]
    pub summary: String,
    /// Where the full report is (`worklog:<sid>:turn:<seq>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub works_created: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub works_absorbed: Vec<String>,
    #[serde(default)]
    pub cost: ActivationCost,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    /// The submitted Goal result, as accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LastActivation {
    pub epoch: u64,
    pub seq: u64,
    pub at_ms: u64,
    #[serde(default)]
    pub reasons: Vec<ActivationReason>,
    pub outcome: ActivationOutcome,
    pub decision: ActivationDecision,
    #[serde(default)]
    pub summary: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalDistance {
    pub text: String,
    #[serde(default)]
    pub observed_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    /// What is not known (never "no change").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unknowns: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalQuestion {
    pub text: String,
    pub at_ms: u64,
    pub epoch: u64,
    pub activation: u64,
    /// The notification that asks it (none: not sent, or no notification
    /// service).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<GoalQuestionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
    /// The maintenance candidate this question confirms (§5.1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GoalQuestionStatus {
    Open,
    Answered,
    Dismissed,
    Expired,
    Canceled,
}

impl GoalQuestionStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Answered => "answered",
            Self::Dismissed => "dismissed",
            Self::Expired => "expired",
            Self::Canceled => "canceled",
        }
    }
}

/// A Work of the Goal not absorbed yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalWork {
    pub sid: String,
    #[serde(default)]
    pub objective: String,
    pub epoch: u64,
    pub created_in_activation: u64,
    pub goal_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecentWork {
    pub sid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default)]
    pub one_line_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report_ref: Option<String>,
    /// `(epoch, activation)` that absorbed it.
    pub absorbed_in: (u64, u64),
}

/// Usage of the current statistics period.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalUsage {
    pub period_start_ms: u64,
    /// Tokens of the Goal Session's Activations.
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub activations: u64,
    /// Work created in the period.
    #[serde(default)]
    pub works: u64,
    /// Tokens of the Work absorbed in the period.
    #[serde(default)]
    pub work_tokens: u64,
}

/// `state.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalState {
    pub schema: String,
    pub goal_id: String,
    /// Monotonic; every published change.
    pub rev: u64,
    pub epoch: u64,
    /// Goal revision the last Activation worked with.
    #[serde(default)]
    pub goal_revision_seen: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_activation: Option<LastActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<GoalDistance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending: Vec<String>,
    /// Questions for the user: sent as notifications when the host has a
    /// notification service; answers come back as `answer` Activations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<GoalQuestion>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_works: Vec<GoalWork>,
    /// Work whose end was taken in by an Activation (any epoch; bounded).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absorbed: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recent_works: Vec<RecentWork>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub current_skills: Vec<GoalSkill>,
    #[serde(default)]
    pub usage: GoalUsage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<Value>,
    /// The consolidation Goal's typed maintenance state (receipts,
    /// candidates, consolidation summary); committed with the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintenance: Option<MaintenanceState>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl GoalState {
    pub fn initial(goal: &GoalDefinition, epoch: u64, now_ms: u64) -> Self {
        Self {
            schema: GOAL_STATE_SCHEMA.to_string(),
            goal_id: goal.goal_id.clone(),
            rev: 1,
            epoch,
            goal_revision_seen: 0,
            last_activation: None,
            distance: None,
            pending: Vec::new(),
            questions: Vec::new(),
            active_works: Vec::new(),
            absorbed: Vec::new(),
            recent_works: Vec::new(),
            current_skills: goal.initial_skills.clone(),
            usage: GoalUsage {
                period_start_ms: now_ms,
                ..Default::default()
            },
            last_error: None,
            maintenance: None,
            updated_at_ms: now_ms,
        }
    }

    /// The state a new epoch starts with.
    pub fn for_epoch(&self, goal: &GoalDefinition, epoch: u64, keep: StateKeep, now_ms: u64) -> Self {
        let mut next = match keep {
            StateKeep::All => self.clone(),
            StateKeep::Facts => {
                let mut s = GoalState::initial(goal, epoch, now_ms);
                s.active_works = self.active_works.clone();
                s.absorbed = self.absorbed.clone();
                s.recent_works = self.recent_works.clone();
                s.usage = self.usage.clone();
                s.goal_revision_seen = self.goal_revision_seen;
                s.maintenance = self.maintenance.clone();
                s
            }
        };
        next.epoch = epoch;
        next.rev = self.rev + 1;
        next.updated_at_ms = now_ms;
        next
    }

    pub fn is_absorbed(&self, sid: &str) -> bool {
        self.absorbed.iter().any(|s| s == sid)
    }

    pub fn maintenance_mut(&mut self) -> &mut MaintenanceState {
        self.maintenance.get_or_insert_with(MaintenanceState::default)
    }
}

/// Stage of a consolidation receipt (§4.1): each step is re-entrant, a
/// crash resumes from the stage it reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStage {
    /// The batch was read (and, with `plan`, the plan persisted).
    Prepared,
    /// The Memory envelope is committed under `operation_id`.
    MemoryCommitted,
    /// Candidates are published to the Goal State.
    Published,
    /// Cleanup of the disposed bodies ran (retried on its own when it
    /// failed: `cleanup_pending`).
    Completed,
    /// The commit was refused for good (a stale lease); needs attention.
    Failed,
}

/// One batch of one Activation of the consolidation Goal (§4.1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ConsolidationReceipt {
    /// Stable operation id = Memory idempotency key.
    pub operation_id: String,
    pub epoch: u64,
    pub activation: u64,
    pub batch_id: String,
    pub graph_seq: u64,
    /// Materials that may be disposed by this batch.
    #[serde(default)]
    pub refs: Vec<String>,
    /// The complete `Batch` (replayed into the commit as it was read).
    pub batch: Value,
    /// The complete `ConsolidationCommit` once submitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Value>,
    pub stage: ReceiptStage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_occasion: Option<String>,
    #[serde(default)]
    pub operations: u64,
    #[serde(default)]
    pub dispositions: u64,
    #[serde(default)]
    pub cleanup_pending: bool,
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl ConsolidationReceipt {
    pub fn is_open(&self) -> bool {
        !matches!(self.stage, ReceiptStage::Completed | ReceiptStage::Failed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Goal,
    Schedule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CandidateStatus {
    Proposed,
    NeedsConfirmation,
    Approved,
    Materializing,
    Created,
    Linked,
    Rejected,
    Expired,
    Blocked,
}

/// A Goal / schedule the consolidation discovered (§5.1). P0 freezes the
/// contract; discovery and confirmation are P1.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceCandidate {
    pub candidate_id: String,
    pub revision: u64,
    pub kind: CandidateKind,
    pub semantic_key: String,
    pub title: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    /// `explicit | inferred`.
    pub intent_basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub proposed: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_budget: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_effects: Vec<String>,
    pub status: CandidateStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub question_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linked_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<u64>,
}

/// `GoalState.maintenance` of the consolidation Goal.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceState {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipts: Vec<ConsolidationReceipt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<MaintenanceCandidate>,
    /// Last commit that really changed or disposed something.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_consolidation_at_ms: Option<u64>,
    #[serde(default)]
    pub consolidations: u64,
    #[serde(default)]
    pub operations: u64,
    #[serde(default)]
    pub dispositions: u64,
    /// Batches read but never committed (recovery dropped them; the
    /// material stays for the next batch).
    #[serde(default)]
    pub abandoned_batches: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl MaintenanceState {
    pub fn open_receipts(&self) -> impl Iterator<Item = &ConsolidationReceipt> {
        self.receipts.iter().filter(|r| r.is_open())
    }

    pub fn receipt_mut(&mut self, operation_id: &str) -> Option<&mut ConsolidationReceipt> {
        self.receipts.iter_mut().find(|r| r.operation_id == operation_id)
    }

    /// Drop the oldest completed receipts beyond the keep bound.
    pub fn trim(&mut self) {
        let completed = self.receipts.iter().filter(|r| !r.is_open()).count();
        let mut extra = completed.saturating_sub(MAINTENANCE_RECEIPTS_KEEP);
        self.receipts.retain(|r| {
            if extra > 0 && !r.is_open() {
                extra -= 1;
                false
            } else {
                true
            }
        });
    }
}

/// A scheduled fire written before it is posted (re-posted with the same
/// key after a crash).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PendingFire {
    pub slot: u64,
    pub key: String,
    pub missed: u64,
}

/// Summary of Memory's pending work as the gate saw it last.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MaintenancePending {
    #[serde(default)]
    pub new: usize,
    #[serde(default)]
    pub due: usize,
    #[serde(default)]
    pub waiting: usize,
    #[serde(default)]
    pub retry_cleanup: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_deadline: Option<String>,
    /// Oldest material not disposed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest_at_ms: Option<u64>,
}

impl MaintenancePending {
    pub fn backlog(&self) -> bool {
        self.new > 0 || self.due > 0
    }
}

/// Deterministic check water marks of the consolidation Goal (§6), kept by
/// the host in `schedule.json`: "checked", as opposed to "consolidated"
/// (`GoalState.maintenance`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MaintenanceGate {
    /// `no_work | no_change | active | mechanical | paused | disabled |
    /// unknown | backoff | period_budget`.
    #[serde(default)]
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_baseline_slot: Option<u64>,
    #[serde(default)]
    pub last_checked_at_ms: u64,
    /// Last time the full deterministic check (`pending_work`) ran.
    #[serde(default)]
    pub last_full_check_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_skip_reason: Option<String>,
    /// Memory snapshot token the full check saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_inputs_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<MaintenancePending>,
    #[serde(default)]
    pub due_reviews: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_review_at: Option<String>,
    #[serde(default)]
    pub unfinished_receipts: usize,
    /// Last run opportunity the gate let through.
    #[serde(default)]
    pub last_run_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_until_ms: Option<u64>,
    #[serde(default)]
    pub backoff_level: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SkippedFire {
    pub slot: u64,
    pub at_ms: u64,
    /// `paused | period_budget | min_interval | not_active`, or a gate
    /// reason of the consolidation Goal (`no_work | no_change | …`).
    pub reason: String,
}

/// `schedule.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalSchedule {
    pub schema: String,
    pub goal_id: String,
    /// `local` (phase 1).
    pub backend: String,
    /// Interval the slots count in (a changed interval starts counting
    /// again).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub every_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_slot: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingFire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_posted_key: Option<String>,
    #[serde(default)]
    pub last_fire_at_ms: u64,
    /// Epoch whose start event (`created / reopened / restarted`) was
    /// posted.
    #[serde(default)]
    pub start_posted_epoch: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<SkippedFire>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// The consolidation Goal's check water marks (§6).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate: Option<MaintenanceGate>,
    #[serde(default)]
    pub updated_at_ms: u64,
}

impl GoalSchedule {
    pub fn new(goal_id: &str, backend: &str) -> Self {
        Self {
            schema: GOAL_SCHEDULE_SCHEMA.to_string(),
            goal_id: goal_id.to_string(),
            backend: backend.to_string(),
            every_secs: None,
            last_slot: None,
            pending: None,
            last_posted_key: None,
            last_fire_at_ms: 0,
            start_posted_epoch: 0,
            skipped: Vec::new(),
            last_error: None,
            gate: None,
            updated_at_ms: 0,
        }
    }

    pub fn skip(&mut self, slot: u64, at_ms: u64, reason: &str) {
        self.skipped.push(SkippedFire {
            slot,
            at_ms,
            reason: reason.to_string(),
        });
        let extra = self.skipped.len().saturating_sub(GOAL_SKIPPED_KEEP);
        self.skipped.drain(..extra);
    }
}

/// `distance` of a Goal result: a sentence, or the structured form.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum DistanceInput {
    Text(String),
    Full {
        text: String,
        #[serde(default)]
        evidence_refs: Vec<String>,
        #[serde(default)]
        unknowns: Vec<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillsChange {
    #[serde(default)]
    pub add: Vec<GoalSkill>,
    /// Skill names.
    #[serde(default)]
    pub remove: Vec<String>,
}

/// `result` of the final `report` of an Activation (checked by the runtime;
/// a result that does not match makes the `report` call fail).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GoalResult {
    /// `no_action | acted | created_work | needs_info`.
    pub decision: ActivationDecision,
    #[serde(default)]
    pub distance: Option<DistanceInput>,
    /// Replaces the open items.
    #[serde(default)]
    pub pending: Option<Vec<String>>,
    /// Questions for the user (replace the open ones; each reaches the
    /// user as a notification).
    #[serde(default)]
    pub questions: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Option<SkillsChange>,
}

impl GoalResult {
    pub fn parse(v: &Value) -> Result<Self, String> {
        let r: GoalResult = serde_json::from_value(v.clone()).map_err(|e| {
            format!(
                "the result of a Goal Activation is {{decision: no_action | acted | created_work | needs_info, distance?, pending?, questions?, skills?: {{add?, remove?}}}}: {e}"
            )
        })?;
        if r.decision == ActivationDecision::Unknown {
            return Err("decision `unknown` is recorded by the runtime; report what you decided".into());
        }
        Ok(r)
    }
}

/// The Activation in progress of a Goal Session (= its open Turn).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OpenActivation {
    pub seq: u64,
    pub goal_revision: u64,
    #[serde(default)]
    pub reasons: Vec<ActivationReason>,
    pub started_at_ms: u64,
    /// Length of the session's `usage.jsonl` when it opened: its tokens are
    /// the records after it.
    #[serde(default)]
    pub usage_offset: u64,
    /// Effective limits, frozen when it opened. Its tokens and runs are
    /// those of the `usage.jsonl` records after `usage_offset`.
    #[serde(default)]
    pub budget: ActivationBudget,
    /// Work whose end it consumed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absorbed: Vec<String>,
}

/// How the Goal Session ends (decided by its Goal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GoalEnding {
    pub end: EpochEnd,
    pub outcome: Outcome,
    #[serde(default)]
    pub reason: String,
    /// The Goal's running Work is stopped too (archive).
    #[serde(default)]
    pub cascade: bool,
}

/// `state.json.goal` of a Goal Session: the Goal State the session owns
/// (committed with the session, published to `state/goals/<id>/state.json`
/// afterwards), Activation records not appended yet, the open Activation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct GoalSessionState {
    pub goal_id: String,
    pub epoch: u64,
    pub state: GoalState,
    /// `state.rev` published last.
    #[serde(default)]
    pub published_rev: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unwritten: Vec<GoalActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<OpenActivation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ending: Option<GoalEnding>,
}

impl GoalSessionState {
    /// Committed changes the Agent State does not have yet.
    pub fn needs_flush(&self) -> bool {
        self.published_rev < self.state.rev || !self.unwritten.is_empty()
    }
}
